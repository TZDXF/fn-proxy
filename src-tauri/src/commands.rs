use crate::{
    auth::NasSession,
    error::{error, Result},
    proxy::{self, ProxyHandle, SessionHub},
    resolver, storage,
    types::*,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, RwLock as StdRwLock,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[derive(Clone)]
struct Credentials {
    fn_id: String,
    username: String,
    password: Zeroizing<String>,
    remember: bool,
}
pub struct AppState {
    pub profile_path: PathBuf,
    pub profile: StdMutex<Profile>,
    pub session: SessionHub,
    pub info: StdRwLock<ConnectionInfo>,
    pub proxies: Mutex<Vec<ProxyHandle>>,
    pub counter: Arc<AtomicU64>,
    pub operation: Mutex<()>,
    credentials: Mutex<Option<Credentials>>,
    monitor: Mutex<Option<CancellationToken>>,
    logs: StdMutex<VecDeque<LogEntry>>,
}
impl AppState {
    pub fn new(profile_path: PathBuf, profile: Profile) -> Self {
        Self {
            profile_path,
            profile: StdMutex::new(profile),
            session: Arc::new(tokio::sync::RwLock::new(None)),
            info: StdRwLock::new(ConnectionInfo::default()),
            proxies: Mutex::new(vec![]),
            counter: Arc::new(AtomicU64::new(0)),
            operation: Mutex::new(()),
            credentials: Mutex::new(None),
            monitor: Mutex::new(None),
            logs: StdMutex::new(VecDeque::new()),
        }
    }
    fn log(&self, app: &AppHandle, level: &str, message: impl Into<String>) {
        let entry = LogEntry {
            time: resolver::millis(),
            level: level.to_owned(),
            message: message.into(),
        };
        let mut logs = self.logs.lock().unwrap();
        logs.push_back(entry.clone());
        if logs.len() > 200 {
            logs.pop_front();
        }
        let _ = app.emit("fn-proxy:log", entry);
    }
    fn profile(&self) -> Profile {
        self.profile.lock().unwrap().clone()
    }
}
async fn stop_internal(state: &AppState) {
    let handles = std::mem::take(&mut *state.proxies.lock().await);
    proxy::stop(handles).await;
}
async fn establish(
    app: &AppHandle,
    state: &Arc<AppState>,
    input: ConnectInput,
) -> Result<ConnectionInfo> {
    let fn_id = normalize_fnid(&input.fn_id)?;
    let username = input.username.trim().to_owned();
    if username.is_empty() {
        return Err(error("请输入 NAS 用户名"));
    }
    let password = match input.password.filter(|s| !s.is_empty()) {
        Some(s) => Zeroizing::new(s),
        None => storage::saved_password(&fn_id, &username)?,
    };
    stop_internal(state).await;
    if let Some(cancel) = state.monitor.lock().await.take() {
        cancel.cancel();
    }
    if let Some(previous) = state.session.write().await.take() {
        previous.rpc.lock().await.close().await;
    }
    *state.credentials.lock().await = None;
    *state.info.write().unwrap() = ConnectionInfo {
        fn_id: fn_id.clone(),
        username: username.clone(),
        message: "正在解析 FN ID 并连接远程入口…".to_owned(),
        ..Default::default()
    };
    state.log(
        app,
        "info",
        "正在解析 FN ID（仅选择远程中继，不使用内网 IP）",
    );
    let base = resolver::resolve(&fn_id).await?;
    state.log(
        app,
        "info",
        "远程入口已找到，正在执行加密登录和服务凭据交换",
    );
    let nas = NasSession::login(
        base,
        &fn_id,
        &username,
        &password,
        input.otp.as_deref(),
        input.remember,
    )
    .await?;
    *state.credentials.lock().await = Some(Credentials {
        fn_id,
        username,
        password,
        remember: input.remember,
    });
    *state.session.write().await = Some(nas.clone());
    *state.info.write().unwrap() = nas.info.clone();
    state.log(
        app,
        "success",
        "NAS 自动登录成功，entry-token 已在后端内存中就绪",
    );
    spawn_monitor(app.clone(), state.clone(), nas.clone()).await;
    Ok(nas.info.clone())
}
async fn spawn_monitor(app: AppHandle, state: Arc<AppState>, mut current: Arc<NasSession>) {
    let cancel = CancellationToken::new();
    *state.monitor.lock().await = Some(cancel.clone());
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        tick.tick().await;
        let mut refreshed = tokio::time::Instant::now();
        loop {
            tokio::select! {_=cancel.cancelled()=>break,_=tick.tick()=>{}}
            let alive = {
                let mut rpc = current.rpc.lock().await;
                tokio::select! {_=cancel.cancelled()=>break,result=rpc.heartbeat()=>result}
            };
            if alive.is_ok() {
                if refreshed.elapsed() >= Duration::from_secs(15 * 60) {
                    let updated = tokio::select! {_=cancel.cancelled()=>break,result=current.refresh_entry_token()=>result};
                    if updated.is_ok() {
                        state.log(&app, "info", "服务访问凭据已更新（不记录凭据值）");
                    }
                    refreshed = tokio::time::Instant::now();
                }
                continue;
            }
            let _guard = state.operation.lock().await;
            if cancel.is_cancelled() {
                break;
            }
            state.log(&app, "warn", "NAS 认证连接中断，尝试一次自动重新登录");
            state.info.write().unwrap().connected = false;
            let credentials = state.credentials.lock().await.clone();
            let Some(credentials) = credentials else {
                break;
            };
            let reconnect = async {
                let base = resolver::resolve(&credentials.fn_id).await?;
                NasSession::login(
                    base,
                    &credentials.fn_id,
                    &credentials.username,
                    &credentials.password,
                    None,
                    credentials.remember,
                )
                .await
            };
            let recovered = tokio::select! {_=cancel.cancelled()=>break,result=reconnect=>result};
            match recovered {
                Ok(nas) => {
                    current = nas;
                    *state.session.write().await = Some(current.clone());
                    *state.info.write().unwrap() = current.info.clone();
                    state.log(
                        &app,
                        "success",
                        "认证连接已恢复，现有本地监听继续使用新凭据",
                    );
                    refreshed = tokio::time::Instant::now();
                }
                Err(e) => {
                    *state.session.write().await = None;
                    state.info.write().unwrap().message = e.to_string();
                    state.log(
                        &app,
                        "error",
                        "自动重新登录失败；为避免锁定账号，已停止重试，请手动测试连接",
                    );
                    break;
                }
            }
        }
    });
}
#[tauri::command]
pub fn get_bootstrap(state: State<'_, Arc<AppState>>) -> Bootstrap {
    let profile = state.profile();
    let has_saved_password =
        profile.remember && storage::saved_password(&profile.fn_id, &profile.username).is_ok();
    Bootstrap {
        profile,
        has_saved_password,
    }
}
#[tauri::command]
pub async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<AppSnapshot> {
    let listeners = state
        .proxies
        .lock()
        .await
        .iter()
        .map(|h| h.info.clone())
        .collect::<Vec<_>>();
    Ok(AppSnapshot {
        connection: state.info.read().unwrap().clone(),
        proxy: ProxyStatus {
            running: !listeners.is_empty(),
            listeners,
            requests: state.counter.load(Ordering::Relaxed),
        },
    })
}
#[tauri::command]
pub fn get_logs(state: State<'_, Arc<AppState>>) -> Vec<LogEntry> {
    state.logs.lock().unwrap().iter().cloned().collect()
}
#[tauri::command]
pub async fn connect_nas(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    input: ConnectInput,
) -> Result<ConnectionInfo> {
    let _guard = state.operation.lock().await;
    match establish(&app, &state, input).await {
        Ok(v) => Ok(v),
        Err(e) => {
            state.info.write().unwrap().message = e.to_string();
            state.log(&app, "error", e.to_string());
            Err(e)
        }
    }
}
#[tauri::command]
pub async fn disconnect_nas(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<()> {
    let _guard = state.operation.lock().await;
    stop_internal(&state).await;
    if let Some(cancel) = state.monitor.lock().await.take() {
        cancel.cancel();
    }
    if let Some(session) = state.session.write().await.take() {
        session.rpc.lock().await.close().await;
    }
    *state.credentials.lock().await = None;
    *state.info.write().unwrap() = ConnectionInfo::default();
    state.log(&app, "info", "已断开连接并停止全部代理监听");
    Ok(())
}
#[tauri::command]
pub async fn save_login(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    mut profile: Profile,
) -> Result<()> {
    let _guard = state.operation.lock().await;
    profile.fn_id = normalize_fnid(&profile.fn_id)?;
    validate_routes(&profile.services, &profile.fn_id)?;
    let credentials = state
        .credentials
        .lock()
        .await
        .clone()
        .ok_or_else(|| error("请先测试连接成功，再保存登录"))?;
    if credentials.fn_id != profile.fn_id || credentials.username != profile.username {
        return Err(error("当前表单与已登录账号不一致，请重新测试连接"));
    }
    if profile.auto_connect && !profile.remember {
        return Err(error("启动时自动登录需要先选择安全保存密码"));
    }
    let previous = state.profile();
    if profile.remember {
        storage::store_password(&profile.fn_id, &profile.username, &credentials.password)?;
    } else {
        storage::delete_password(&profile.fn_id, &profile.username)?;
    }
    storage::save_profile(&state.profile_path, &profile)?;
    if (previous.fn_id != profile.fn_id || previous.username != profile.username)
        && previous.remember
    {
        let _ = storage::delete_password(&previous.fn_id, &previous.username);
    }
    if let Some(current) = state.credentials.lock().await.as_mut() {
        current.remember = profile.remember;
    }
    *state.profile.lock().unwrap() = profile;
    state.log(
        &app,
        "success",
        "登录配置已保存；密码仅保存到 Windows 凭据管理器",
    );
    Ok(())
}
#[tauri::command]
pub async fn forget_login(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<()> {
    let _guard = state.operation.lock().await;
    let mut profile = state.profile();
    storage::delete_password(&profile.fn_id, &profile.username)?;
    profile.remember = false;
    profile.auto_connect = false;
    storage::save_profile(&state.profile_path, &profile)?;
    if let Some(current) = state.credentials.lock().await.as_mut() {
        current.remember = false;
    }
    *state.profile.lock().unwrap() = profile;
    state.log(&app, "info", "已删除系统凭据管理器中的已保存密码");
    Ok(())
}
#[tauri::command]
pub async fn discover_services(state: State<'_, Arc<AppState>>) -> Result<Vec<DiscoveredService>> {
    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("请先登录 NAS"))?;
    session.discover().await
}
#[tauri::command]
pub async fn probe_service(
    state: State<'_, Arc<AppState>>,
    route: ServiceRoute,
) -> Result<RouteProbe> {
    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("请先登录 NAS"))?;
    let url = validate_upstream(&route.upstream, &session.info.fn_id)?;
    let token = session.entry_token.read().await.to_string();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client
        .get(url)
        .header("Cookie", format!("entry-token={token}"))
        .send()
        .await?;
    let status = response.status().as_u16();
    Ok(RouteProbe {
        status,
        reachable: (200..400).contains(&status),
        message: match status {
            200..=399 => "服务入口可达；具体 API 可能还需要应用自己的认证",
            401 => "已收到服务响应，但该应用可能需要自身登录或 API Key",
            403 => "访问被拒绝：需检查入口凭据及应用权限",
            _ => "服务返回非成功状态，请检查 NAS 服务",
        }
        .to_owned(),
    })
}
#[tauri::command]
pub async fn refresh_session(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<()> {
    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("请先登录 NAS"))?;
    session.refresh_entry_token().await?;
    state.log(&app, "success", "服务访问凭据已更新");
    Ok(())
}
#[tauri::command]
pub async fn start_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    services: Vec<ServiceRoute>,
) -> Result<ProxyStatus> {
    let _guard = state.operation.lock().await;
    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("请先测试连接并登录 NAS"))?;
    if !state.proxies.lock().await.is_empty() {
        return Err(error("请先停止代理，再修改服务映射"));
    }
    state.counter.store(0, Ordering::Relaxed);
    let handles = proxy::start(
        &services,
        &session.info.fn_id,
        state.session.clone(),
        state.counter.clone(),
    )
    .await?;
    let listeners = handles.iter().map(|h| h.info.clone()).collect();
    *state.proxies.lock().await = handles;
    state.log(
        &app,
        "success",
        "本地代理已启动，仅监听 127.0.0.1；请求将自动附加服务凭据",
    );
    Ok(ProxyStatus {
        running: true,
        listeners,
        requests: 0,
    })
}
#[tauri::command]
pub async fn stop_proxy(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<()> {
    let _guard = state.operation.lock().await;
    stop_internal(&state).await;
    state.log(&app, "info", "全部本地代理监听已停止");
    Ok(())
}
pub fn auto_connect(app: AppHandle, state: Arc<AppState>) {
    let profile = state.profile();
    if !profile.auto_connect || !profile.remember {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let _guard = state.operation.lock().await;
        let input = ConnectInput {
            fn_id: profile.fn_id,
            username: profile.username,
            password: None,
            otp: None,
            remember: true,
        };
        if let Err(e) = establish(&app, &state, input).await {
            state.info.write().unwrap().message = e.to_string();
            state.log(
                &app,
                "error",
                "启动时自动登录失败，请检查网络或重新输入凭据",
            );
        }
    });
}
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    if let Ok(mut monitor) = state.monitor.try_lock() {
        if let Some(cancel) = monitor.take() {
            cancel.cancel();
        }
    }
    let maybe_proxies = state.proxies.try_lock();
    if let Ok(proxies) = maybe_proxies {
        for handle in proxies.iter() {
            handle.cancel.cancel();
        }
    }
}
