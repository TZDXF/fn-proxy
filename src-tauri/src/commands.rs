use crate::{
    auth::NasSession,
    error::{error, error_with, Result},
    proxy::{self, ProxyHandle, SessionHub},
    resolver, service_sync, storage,
    text::Text,
    types::*,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
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
pub struct ConnectionState {
    pub id: String,
    saved: StdMutex<bool>,
    pub profile: StdMutex<Profile>,
    pub session: SessionHub,
    pub info: StdRwLock<ConnectionInfo>,
    pub proxies: Mutex<Vec<ProxyHandle>>,
    pub counter: Arc<AtomicU64>,
    pub operation: Mutex<()>,
    credentials: Mutex<Option<Credentials>>,
    monitor: Mutex<Option<CancellationToken>>,
    domain_warning: StdMutex<Vec<Text>>,
    logs: Arc<StdMutex<VecDeque<LogEntry>>>,
}
impl ConnectionState {
    fn new(profile: Profile, saved: bool, logs: Arc<StdMutex<VecDeque<LogEntry>>>) -> Self {
        Self {
            id: profile.id.clone(),
            saved: StdMutex::new(saved),
            profile: StdMutex::new(profile),
            session: Arc::new(tokio::sync::RwLock::new(None)),
            info: StdRwLock::new(ConnectionInfo::default()),
            proxies: Mutex::new(vec![]),
            counter: Arc::new(AtomicU64::new(0)),
            operation: Mutex::new(()),
            credentials: Mutex::new(None),
            monitor: Mutex::new(None),
            domain_warning: StdMutex::new(Vec::new()),
            logs,
        }
    }
    fn log(&self, app: &AppHandle, level: &str, message: Text) {
        let info = self.info.read().unwrap().clone();
        let profile = self.profile();
        let fn_id = if info.fn_id.is_empty() {
            &profile.fn_id
        } else {
            &info.fn_id
        };
        let label = if fn_id.is_empty() {
            self.id.clone()
        } else {
            format!("{fn_id} · {}", self.id)
        };
        let entry = LogEntry {
            time: resolver::millis(),
            level: level.to_owned(),
            label,
            message,
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
pub struct AppState {
    allow_lan_access: AtomicBool,
    pub profile_path: PathBuf,
    connections: StdMutex<BTreeMap<String, Arc<ConnectionState>>>,
    operation: Mutex<()>,
    logs: Arc<StdMutex<VecDeque<LogEntry>>>,
}
impl AppState {
    pub fn new(profile_path: PathBuf, workspace: WorkspaceProfiles) -> Self {
        let logs = Arc::new(StdMutex::new(VecDeque::new()));
        let mut connections = BTreeMap::new();
        for profile in workspace.profiles {
            connections.insert(
                profile.id.clone(),
                Arc::new(ConnectionState::new(profile, true, logs.clone())),
            );
        }
        if connections.is_empty() {
            let profile = Profile {
                id: "default".to_owned(),
                ..Default::default()
            };
            connections.insert(
                profile.id.clone(),
                Arc::new(ConnectionState::new(profile, false, logs.clone())),
            );
        }
        Self {
            allow_lan_access: AtomicBool::new(workspace.allow_lan_access),
            profile_path,
            connections: StdMutex::new(connections),
            operation: Mutex::new(()),
            logs,
        }
    }
    fn all(&self) -> Vec<Arc<ConnectionState>> {
        self.connections.lock().unwrap().values().cloned().collect()
    }
    fn connection(&self, id: &str) -> Result<Arc<ConnectionState>> {
        self.connections
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| error("workspace.connectionMissing"))
    }
    fn ensure_connection(&self, id: &str) -> Result<Arc<ConnectionState>> {
        if id.is_empty()
            || id.len() > 64
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(error("workspace.connectionIdInvalid"));
        }
        let mut connections = self.connections.lock().unwrap();
        Ok(connections
            .entry(id.to_owned())
            .or_insert_with(|| {
                Arc::new(ConnectionState::new(
                    Profile {
                        id: id.to_owned(),
                        ..Default::default()
                    },
                    false,
                    self.logs.clone(),
                ))
            })
            .clone())
    }
    fn password_used_by_other(&self, profile: &Profile) -> bool {
        self.all().iter().any(|other| {
            let p = other.profile();
            other.id != profile.id
                && *other.saved.lock().unwrap()
                && p.remember
                && p.fn_id == profile.fn_id
                && p.username == profile.username
        })
    }
    fn persist(&self, replacement: Option<&Profile>, removed: Option<&str>) -> Result<()> {
        self.persist_with_access(
            replacement,
            removed,
            self.allow_lan_access.load(Ordering::Relaxed),
        )
    }
    fn persist_with_access(
        &self,
        replacement: Option<&Profile>,
        removed: Option<&str>,
        allow_lan_access: bool,
    ) -> Result<()> {
        let mut profiles: Vec<Profile> = self
            .all()
            .into_iter()
            .filter(|c| {
                *c.saved.lock().unwrap()
                    && Some(c.id.as_str()) != removed
                    && replacement.is_none_or(|p| p.id != c.id)
            })
            .map(|c| c.profile())
            .collect();
        if let Some(profile) = replacement {
            profiles.push(profile.clone());
        }
        storage::save_profiles(
            &self.profile_path,
            &WorkspaceProfiles {
                profiles,
                allow_lan_access,
            },
        )
    }
}
async fn stop_internal(state: &ConnectionState) {
    let handles = std::mem::take(&mut *state.proxies.lock().await);
    proxy::stop(handles).await;
}
// Call while holding workspace and connection operation locks, in that order.
async fn apply_domain_inventory(
    app: &AppHandle,
    manager: &AppState,
    state: &ConnectionState,
    session: &Arc<NasSession>,
    inventory: &ServiceInventory,
) -> Result<()> {
    if !state
        .session
        .read()
        .await
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, session))
    {
        return Ok(()); // The discovery belongs to a disconnected/replaced session.
    }
    let mut profile = state.profile();
    if profile.services.is_empty()
        || (!profile.fn_id.is_empty() && profile.fn_id != session.info.fn_id)
    {
        return Ok(());
    }
    let result = service_sync::reconcile(&profile.services, inventory, &session.info.fn_id);
    report_domain_warnings(app, state, &result.warnings);
    if result.changed == 0 {
        return Ok(());
    }
    profile.services = result.routes;
    validate_routes(&profile.services, &session.info.fn_id)?;
    // Validate targets, persist, then commit synchronously. Cancellation/save errors cannot
    // leave the stored profile, runtime targets and in-memory cache at different versions.
    let mut proxies = state.proxies.lock().await;
    proxy::update_upstreams(&mut proxies, &profile.services, &session.info.fn_id, || {
        if *state.saved.lock().unwrap() {
            manager.persist(Some(&profile), None)
        } else {
            Ok(())
        }
    })?;
    *state.profile.lock().unwrap() = profile;
    state.log(
        app,
        "info",
        Text::with(
            "logs.domainsUpdated",
            [("count", result.changed.to_string())],
        ),
    );
    Ok(())
}
fn report_domain_warnings(app: &AppHandle, state: &ConnectionState, warnings: &[Text]) {
    let changed = {
        let mut previous = state.domain_warning.lock().unwrap();
        let changed = *previous != warnings;
        *previous = warnings.to_vec();
        changed
    };
    if changed {
        for warning in warnings {
            state.log(app, "warn", warning.clone());
        }
    }
}
async fn refresh_domains(
    app: &AppHandle,
    manager: &AppState,
    state: &ConnectionState,
    session: &Arc<NasSession>,
) -> Result<()> {
    if state.profile().services.is_empty() {
        return Ok(());
    }
    let inventory = session.domain_inventory().await?;
    let _workspace_guard = manager.operation.lock().await;
    let _guard = state.operation.lock().await;
    apply_domain_inventory(app, manager, state, session, &inventory).await
}
async fn establish(
    app: &AppHandle,
    state: &Arc<ConnectionState>,
    input: ConnectInput,
) -> Result<ConnectionInfo> {
    let fn_id = normalize_fnid(&input.fn_id)?;
    let username = input.username.trim().to_owned();
    if username.is_empty() {
        return Err(error("workspace.usernameRequired"));
    }
    let password = match input.password.filter(|s| !s.is_empty()) {
        Some(s) => Zeroizing::new(s),
        None => {
            let profile = state.profile();
            if !profile.remember || profile.fn_id != fn_id || profile.username != username {
                return Err(error("workspace.noSavedPassword"));
            }
            storage::saved_password(&fn_id, &username)?
        }
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
        message: Text::new("logs.resolvingEntry"),
        ..Default::default()
    };
    state.log(app, "info", Text::new("logs.resolving"));
    let base = resolver::resolve(&fn_id).await?;
    state.log(app, "info", Text::new("logs.relayFound"));
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
    state.log(app, "success", Text::new("logs.loginSuccess"));
    spawn_monitor(app.clone(), state.clone(), nas.clone()).await;
    Ok(nas.info.clone())
}
async fn spawn_monitor(app: AppHandle, state: Arc<ConnectionState>, mut current: Arc<NasSession>) {
    let cancel = CancellationToken::new();
    *state.monitor.lock().await = Some(cancel.clone());
    let manager = app.state::<Arc<AppState>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let initial = tokio::select! { _=cancel.cancelled()=>return, result=refresh_domains(&app, &manager, &state, &current)=>result };
        if initial.is_err() {
            report_domain_warnings(&app, &state, &[Text::new("logs.domainReadFailed")]);
        }
        let mut domains_refreshed = tokio::time::Instant::now();
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
                if domains_refreshed.elapsed() >= Duration::from_secs(60) {
                    let updated = tokio::select! { _=cancel.cancelled()=>break, result=refresh_domains(&app, &manager, &state, &current)=>result };
                    if updated.is_err() {
                        report_domain_warnings(&app, &state, &[Text::new("logs.domainSyncFailed")]);
                    }
                    domains_refreshed = tokio::time::Instant::now();
                }
                if refreshed.elapsed() >= Duration::from_secs(15 * 60) {
                    let updated = tokio::select! {_=cancel.cancelled()=>break,result=current.refresh_entry_token()=>result};
                    if updated.is_ok() {
                        state.log(&app, "info", Text::new("logs.credentialsRefreshed"));
                    }
                    refreshed = tokio::time::Instant::now();
                }
                continue;
            }
            let _guard = state.operation.lock().await;
            if cancel.is_cancelled() {
                break;
            }
            state.log(&app, "warn", Text::new("logs.reconnecting"));
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
                    state.log(&app, "success", Text::new("logs.reconnected"));
                    refreshed = tokio::time::Instant::now();
                    drop(_guard);
                    let updated = tokio::select! { _=cancel.cancelled()=>break, result=refresh_domains(&app, &manager, &state, &current)=>result };
                    if updated.is_err() {
                        report_domain_warnings(
                            &app,
                            &state,
                            &[Text::new("logs.domainSyncAfterReconnectFailed")],
                        );
                    }
                    domains_refreshed = tokio::time::Instant::now();
                }
                Err(e) => {
                    *state.session.write().await = None;
                    state.info.write().unwrap().message = e.text();
                    state.log(&app, "error", Text::new("logs.reconnectFailed"));
                    break;
                }
            }
        }
    });
}
#[tauri::command]
pub fn get_bootstrap(state: State<'_, Arc<AppState>>) -> Bootstrap {
    Bootstrap {
        allow_lan_access: state.allow_lan_access.load(Ordering::Relaxed),
        profiles: state
            .all()
            .into_iter()
            .map(|connection| {
                let profile = connection.profile();
                let has_saved_password = profile.remember
                    && storage::saved_password(&profile.fn_id, &profile.username).is_ok();
                SavedProfile {
                    profile,
                    has_saved_password,
                }
            })
            .collect(),
    }
}
async fn set_lan_access(manager: &AppState, enabled: bool) -> Result<()> {
    let _workspace_guard = manager.operation.lock().await;
    if manager.allow_lan_access.load(Ordering::Relaxed) == enabled {
        return Ok(());
    }
    for connection in manager.all() {
        if !connection.proxies.lock().await.is_empty() {
            return Err(error("workspace.stopProxiesBeforeLan"));
        }
    }
    // Save first: a failed write must not change the effective listening policy.
    manager.persist_with_access(None, None, enabled)?;
    manager.allow_lan_access.store(enabled, Ordering::Relaxed);
    Ok(())
}
#[tauri::command]
pub async fn set_allow_lan_access(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<()> {
    set_lan_access(state.inner(), enabled).await
}
#[tauri::command]
pub async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<AppSnapshot> {
    let mut connections = vec![];
    for state in state.all() {
        let listeners: Vec<_> = state
            .proxies
            .lock()
            .await
            .iter()
            .map(|h| h.info.clone())
            .collect();
        connections.push(ConnectionSnapshot {
            id: state.id.clone(),
            connection: state.info.read().unwrap().clone(),
            services: state.profile().services,
            proxy: ProxyStatus {
                running: !listeners.is_empty(),
                listeners,
                requests: state.counter.load(Ordering::Relaxed),
            },
        });
    }
    Ok(AppSnapshot { connections })
}
#[tauri::command]
pub fn get_logs(state: State<'_, Arc<AppState>>) -> Vec<LogEntry> {
    state.logs.lock().unwrap().iter().cloned().collect()
}
#[tauri::command]
pub async fn connect_nas(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    input: ConnectInput,
) -> Result<ConnectionInfo> {
    let manager = state.inner().clone();
    let state = manager.ensure_connection(&connection_id)?;

    let _guard = state.operation.lock().await;
    match establish(&app, &state, input).await {
        Ok(v) => Ok(v),
        Err(e) => {
            state.info.write().unwrap().message = e.text();
            state.log(&app, "error", e.text());
            Err(e)
        }
    }
}
#[tauri::command]
pub async fn disconnect_nas(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<()> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

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
    state.log(&app, "info", Text::new("logs.disconnected"));
    Ok(())
}
async fn persist_login_profile(
    manager: &AppState,
    connection_id: String,
    mut profile: Profile,
    password: Option<String>,
) -> Result<bool> {
    // Wrap submitted secrets before validation so all return paths zeroize them.
    let password = password.filter(|p| !p.is_empty()).map(Zeroizing::new);
    let _workspace_guard = manager.operation.lock().await;
    let state = manager.ensure_connection(&connection_id)?;
    let _guard = state.operation.lock().await;
    profile.id = connection_id;
    profile.fn_id = normalize_fnid(&profile.fn_id)?;
    profile.username = profile.username.trim().to_owned();
    if profile.username.is_empty() {
        return Err(error("workspace.usernameRequired"));
    }
    validate_routes(&profile.services, &profile.fn_id)?;
    if profile.auto_connect && !profile.remember {
        return Err(error("workspace.autoConnectNeedsRemember"));
    }
    let credentials = state
        .credentials
        .lock()
        .await
        .clone()
        .filter(|credentials| {
            credentials.fn_id == profile.fn_id && credentials.username == profile.username
        });
    let previous = state.profile();
    if profile.remember {
        let password = password
            .or_else(|| credentials.as_ref().map(|c| c.password.clone()))
            .or_else(|| storage::saved_password(&profile.fn_id, &profile.username).ok())
            .ok_or_else(|| error("workspace.passwordRequiredToSave"))?;
        storage::store_password(&profile.fn_id, &profile.username, &password)?;
    } else if !manager.password_used_by_other(&profile) {
        storage::delete_password(&profile.fn_id, &profile.username)?;
    }
    manager.persist(Some(&profile), None)?;
    *state.saved.lock().unwrap() = true;
    if (previous.fn_id != profile.fn_id || previous.username != profile.username)
        && previous.remember
        && !manager.password_used_by_other(&previous)
    {
        let _ = storage::delete_password(&previous.fn_id, &previous.username);
    }
    if let Some(current) = state.credentials.lock().await.as_mut() {
        if current.fn_id == profile.fn_id && current.username == profile.username {
            current.remember = profile.remember;
        }
    }
    let password_saved = profile.remember;
    *state.profile.lock().unwrap() = profile;
    Ok(password_saved)
}
#[tauri::command]
pub async fn save_login(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    profile: Profile,
    password: Option<String>,
) -> Result<bool> {
    let password_saved =
        persist_login_profile(state.inner(), connection_id.clone(), profile, password).await?;
    state
        .connection(&connection_id)?
        .log(&app, "success", Text::new("logs.profileSaved"));
    Ok(password_saved)
}
#[tauri::command]
pub async fn forget_login(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<()> {
    let manager = state.inner().clone();
    let _workspace_guard = manager.operation.lock().await;
    let state = manager.connection(&connection_id)?;

    let _guard = state.operation.lock().await;
    let mut profile = state.profile();
    if !manager.password_used_by_other(&profile) {
        storage::delete_password(&profile.fn_id, &profile.username)?;
    }
    profile.remember = false;
    profile.auto_connect = false;
    manager.persist(Some(&profile), None)?;
    *state.saved.lock().unwrap() = true;
    if let Some(current) = state.credentials.lock().await.as_mut() {
        current.remember = false;
    }
    *state.profile.lock().unwrap() = profile;
    state.log(&app, "info", Text::new("logs.passwordForgotten"));
    Ok(())
}
#[tauri::command]
pub async fn discover_services(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<Vec<DiscoveredService>> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("workspace.loginRequired"))?;
    let inventory = session.inventory().await?;
    let _workspace_guard = manager.operation.lock().await;
    let _guard = state.operation.lock().await;
    apply_domain_inventory(&app, &manager, &state, &session, &inventory).await?;
    Ok(inventory.services)
}
#[tauri::command]
pub async fn get_service_inventory(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<crate::types::ServiceInventory> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("workspace.loginRequired"))?;
    let inventory = session.inventory().await?;
    let _workspace_guard = manager.operation.lock().await;
    let _guard = state.operation.lock().await;
    apply_domain_inventory(&app, &manager, &state, &session, &inventory).await?;
    Ok(inventory)
}
#[tauri::command]
pub async fn probe_service(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    route: ServiceRoute,
) -> Result<RouteProbe> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("workspace.loginRequired"))?;
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
        message: Text::new(match status {
            200..=399 => "workspace.probeReachable",
            401 => "workspace.probeUnauthorized",
            403 => "workspace.probeForbidden",
            _ => "workspace.probeFailed",
        }),
    })
}
#[tauri::command]
pub async fn refresh_session(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<()> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("workspace.loginRequired"))?;
    session.refresh_entry_token().await?;
    state.log(&app, "success", Text::new("logs.credentialsUpdated"));
    Ok(())
}
#[tauri::command]
pub async fn start_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    mut services: Vec<ServiceRoute>,
) -> Result<ProxyStatus> {
    let manager = state.inner().clone();
    let _workspace_guard = manager.operation.lock().await;
    let state = manager.connection(&connection_id)?;

    let _guard = state.operation.lock().await;
    let session = state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("workspace.testRequired"))?;
    if !state.proxies.lock().await.is_empty() {
        return Err(error("workspace.stopProxyFirst"));
    }
    validate_routes(&services, &session.info.fn_id)?;
    match session.domain_inventory().await {
        Ok(inventory) => {
            let result = service_sync::reconcile(&services, &inventory, &session.info.fn_id);
            report_domain_warnings(&app, &state, &result.warnings);
            services = result.routes;
        }
        Err(_) => report_domain_warnings(
            &app,
            &state,
            &[Text::new("logs.domainReadBeforeStartFailed")],
        ),
    }
    for other in manager.all().into_iter().filter(|c| c.id != state.id) {
        let proxies = other.proxies.lock().await;
        for route in services.iter().filter(|s| s.enabled) {
            if proxies
                .iter()
                .any(|h| h.info.local_url == format!("http://127.0.0.1:{}/", route.local_port))
            {
                return Err(error_with(
                    "workspace.portInUse",
                    [("port", route.local_port.to_string())],
                ));
            }
        }
    }
    state.counter.store(0, Ordering::Relaxed);
    let handles = proxy::start(
        &services,
        &session.info.fn_id,
        state.session.clone(),
        state.counter.clone(),
        manager.allow_lan_access.load(Ordering::Relaxed),
    )
    .await?;
    let mut profile = state.profile();
    profile.services = services;
    if *state.saved.lock().unwrap() {
        if let Err(e) = manager.persist(Some(&profile), None) {
            proxy::stop(handles).await;
            return Err(e);
        }
    }
    *state.profile.lock().unwrap() = profile;
    let listeners = handles.iter().map(|h| h.info.clone()).collect();
    *state.proxies.lock().await = handles;
    state.log(
        &app,
        "success",
        if manager.allow_lan_access.load(Ordering::Relaxed) {
            Text::new("logs.proxyStartedLan")
        } else {
            Text::new("logs.proxyStartedLocal")
        },
    );
    Ok(ProxyStatus {
        running: true,
        listeners,
        requests: 0,
    })
}
#[tauri::command]
pub async fn update_services(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    mut services: Vec<ServiceRoute>,
    edited_service_id: Option<String>,
) -> Result<ProxyStatus> {
    let manager = state.inner().clone();
    let _workspace_guard = manager.operation.lock().await;
    let state = manager.connection(&connection_id)?;
    let _guard = state.operation.lock().await;
    let mut profile = state.profile();
    let fn_id = if profile.fn_id.is_empty() {
        state.info.read().unwrap().fn_id.clone()
    } else {
        profile.fn_id.clone()
    };
    validate_routes(&services, &fn_id)?;
    let listeners: Vec<_> = state
        .proxies
        .lock()
        .await
        .iter()
        .map(|h| h.info.clone())
        .collect();
    let running = !listeners.is_empty();
    if running {
        // IPC may carry a domain snapshot from before the last background refresh.
        // Keep the live cache for untouched routes, but honor explicit editor changes.
        for route in &mut services {
            if edited_service_id.as_deref() == Some(&route.id) {
                continue;
            }
            if let Some(current) = profile.services.iter().find(|current| {
                current.id == route.id
                    && current.nas_port == route.nas_port
                    && current.local_port == route.local_port
                    && listeners.iter().any(|listener| {
                        listener.local_url == format!("http://127.0.0.1:{}/", route.local_port)
                    })
            }) {
                route.upstream = current.upstream.clone();
            }
        }
    }
    let additions = if running {
        proxy::additional_routes(&services, &listeners)?
    } else {
        vec![]
    };
    for other in manager.all().into_iter().filter(|c| c.id != state.id) {
        let proxies = other.proxies.lock().await;
        for route in services.iter().filter(|s| s.enabled) {
            if proxies
                .iter()
                .any(|h| h.info.local_url == format!("http://127.0.0.1:{}/", route.local_port))
            {
                return Err(error_with(
                    "workspace.portInUse",
                    [("port", route.local_port.to_string())],
                ));
            }
        }
    }
    if !additions.is_empty() && state.session.read().await.is_none() {
        return Err(error("workspace.reconnectRequired"));
    }
    profile.services = services;
    let saved = *state.saved.lock().unwrap();
    let persist = || {
        if saved {
            manager.persist(Some(&profile), None)?;
        }
        Ok(())
    };
    let mut proxies = state.proxies.lock().await;
    if running {
        proxy::apply_routes(
            &mut proxies,
            &profile.services,
            &fn_id,
            state.session.clone(),
            state.counter.clone(),
            manager.allow_lan_access.load(Ordering::Relaxed),
            persist,
        )
        .await?;
    } else {
        persist()?;
    }
    *state.profile.lock().unwrap() = profile;
    let running = !proxies.is_empty();
    let listeners = proxies.iter().map(|h| h.info.clone()).collect();
    state.log(&app, "success", Text::new("logs.servicesUpdated"));
    Ok(ProxyStatus {
        running,
        listeners,
        requests: state.counter.load(Ordering::Relaxed),
    })
}
#[tauri::command]
pub async fn stop_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<()> {
    let manager = state.inner().clone();
    let state = manager.connection(&connection_id)?;

    let _guard = state.operation.lock().await;
    stop_internal(&state).await;
    state.log(&app, "info", Text::new("logs.proxyStopped"));
    Ok(())
}
#[tauri::command]
pub async fn remove_connection(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<()> {
    let _workspace_guard = state.operation.lock().await;
    // Unsaved UI drafts need not have a backend session yet.
    let connection = state
        .connections
        .lock()
        .unwrap()
        .get(&connection_id)
        .cloned();
    let Some(connection) = connection else {
        return Ok(());
    };
    let _guard = connection.operation.lock().await;
    let profile = connection.profile();
    if profile.remember && !state.password_used_by_other(&profile) {
        storage::delete_password(&profile.fn_id, &profile.username)?;
    }
    state.persist(None, Some(&connection_id))?;
    stop_internal(&connection).await;
    if let Some(cancel) = connection.monitor.lock().await.take() {
        cancel.cancel();
    }
    if let Some(session) = connection.session.write().await.take() {
        session.rpc.lock().await.close().await;
    }
    *connection.credentials.lock().await = None;
    state.connections.lock().unwrap().remove(&connection_id);
    connection.log(&app, "info", Text::new("logs.connectionRemoved"));
    Ok(())
}
pub fn auto_connect(app: AppHandle, manager: Arc<AppState>) {
    for state in manager.all() {
        let profile = state.profile();
        if !profile.auto_connect || !profile.remember {
            continue;
        }
        let app = app.clone();
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
                state.info.write().unwrap().message = e.text();
                state.log(&app, "error", Text::new("logs.autoConnectFailed"));
            }
        });
    }
}
pub fn shutdown(app: &AppHandle) {
    let manager = app.state::<Arc<AppState>>().inner().clone();
    for state in manager.all() {
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
}
#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> AppState {
        AppState::new(
            std::env::temp_dir().join(format!(
                "fn-proxy-workspace-test-{}.json",
                rand::random::<u64>()
            )),
            WorkspaceProfiles {
                allow_lan_access: false,
                profiles: vec![
                    Profile {
                        id: "first".to_owned(),
                        fn_id: "nas-a".to_owned(),
                        ..Default::default()
                    },
                    Profile {
                        id: "second".to_owned(),
                        fn_id: "nas-b".to_owned(),
                        ..Default::default()
                    },
                ],
            },
        )
    }
    #[tokio::test]
    async fn saves_a_new_connection_without_credentials_or_a_session() {
        let manager = manager();
        let profile = Profile {
            id: "draft".to_owned(),
            fn_id: "  NAS-C  ".to_owned(),
            username: format!(" fn-proxy-test-{} ", rand::random::<u64>()),
            ..Default::default()
        };
        assert!(
            !persist_login_profile(&manager, "draft".to_owned(), profile, None)
                .await
                .unwrap()
        );
        let state = manager.connection("draft").unwrap();
        assert!(*state.saved.lock().unwrap());
        assert_eq!(state.profile().fn_id, "nas-c");
        assert_eq!(state.profile().username, state.profile().username.trim());
        assert!(!state.info.read().unwrap().connected);
        assert!(state.session.read().await.is_none());
        assert!(state.credentials.lock().await.is_none());
        assert!(state.monitor.lock().await.is_none());
        let workspace = storage::load_profiles(&manager.profile_path).unwrap();
        assert_eq!(workspace.profiles.len(), 3);
        assert!(workspace.profiles.iter().any(|p| p.id == "draft"));
        let _ = std::fs::remove_file(&manager.profile_path);
    }
    #[tokio::test]
    async fn saves_an_existing_disconnected_connection_without_logging_in() {
        let manager = manager();
        let mut profile = manager.connection("first").unwrap().profile();
        profile.username = format!("fn-proxy-test-{}", rand::random::<u64>());
        persist_login_profile(&manager, "first".to_owned(), profile.clone(), None)
            .await
            .unwrap();
        let state = manager.connection("first").unwrap();
        assert_eq!(state.profile().username, profile.username);
        assert!(!state.info.read().unwrap().connected);
        assert!(state.session.read().await.is_none());
        assert!(state.credentials.lock().await.is_none());
        assert_eq!(
            storage::load_profiles(&manager.profile_path)
                .unwrap()
                .profiles
                .len(),
            2
        );
        let _ = std::fs::remove_file(&manager.profile_path);
    }
    #[tokio::test]
    async fn saving_rejects_a_blank_username_without_persisting() {
        let manager = manager();
        let mut profile = manager.connection("first").unwrap().profile();
        profile.username = "  ".to_owned();
        let result = persist_login_profile(&manager, "first".to_owned(), profile, None).await;
        assert_eq!(
            result.unwrap_err().text().code,
            "workspace.usernameRequired"
        );
        assert!(!manager.profile_path.exists());
    }
    #[tokio::test]
    async fn connections_have_independent_sessions_counters_and_listeners() {
        let manager = manager();
        let first = manager.connection("first").unwrap();
        let second = manager.connection("second").unwrap();
        assert!(!Arc::ptr_eq(&first.session, &second.session));
        first.counter.store(12, Ordering::Relaxed);
        assert_eq!(second.counter.load(Ordering::Relaxed), 0);
        first.info.write().unwrap().connected = true;
        assert!(!second.info.read().unwrap().connected);
        // Reserve distinct ephemeral ports before creating the actual proxy listeners.
        let a = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let b = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let first_port = a.local_addr().unwrap().port();
        let second_port = b.local_addr().unwrap().port();
        drop((a, b));
        for (state, port, fn_id) in [
            (&first, first_port, "nas-a"),
            (&second, second_port, "nas-b"),
        ] {
            let routes = vec![ServiceRoute {
                id: "api".to_owned(),
                name: "API".to_owned(),
                nas_port: 8084,
                local_port: port,
                upstream: format!("https://api.{fn_id}.fnos.net/"),
                enabled: true,
            }];
            *state.proxies.lock().await = proxy::start(
                &routes,
                fn_id,
                state.session.clone(),
                state.counter.clone(),
                false,
            )
            .await
            .unwrap();
        }
        stop_internal(&first).await;
        assert!(first.proxies.lock().await.is_empty());
        assert_eq!(second.proxies.lock().await.len(), 1);
        assert!(
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, second_port))
                .await
                .is_err()
        );
        stop_internal(&second).await;
    }
    #[test]
    fn saving_one_connection_preserves_others_and_excludes_unsaved_drafts() {
        let manager = manager();
        manager.ensure_connection("draft").unwrap();
        let mut first = manager.connection("first").unwrap().profile();
        first.username = "updated".to_owned();
        manager.persist(Some(&first), None).unwrap();
        let loaded = storage::load_profiles(&manager.profile_path).unwrap();
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(
            loaded
                .profiles
                .iter()
                .find(|p| p.id == "first")
                .unwrap()
                .username,
            "updated"
        );
        assert!(loaded.profiles.iter().any(|p| p.id == "second"));
        manager.persist(None, Some("first")).unwrap();
        let loaded = storage::load_profiles(&manager.profile_path).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].id, "second");
        std::fs::remove_file(&manager.profile_path).unwrap();
    }
    #[test]
    fn shared_account_credentials_are_kept_until_last_saved_connection_forgets_them() {
        let manager = manager();
        let first = manager.connection("first").unwrap();
        let second = manager.connection("second").unwrap();
        let mut p = first.profile();
        p.username = "admin".to_owned();
        p.remember = true;
        *first.profile.lock().unwrap() = p.clone();
        let mut shared = p.clone();
        shared.id = "second".to_owned();
        *second.profile.lock().unwrap() = shared.clone();
        assert!(manager.password_used_by_other(&p));
        assert!(manager.password_used_by_other(&shared));
        second.profile.lock().unwrap().remember = false;
        assert!(!manager.password_used_by_other(&p));
    }
    #[test]
    fn rejects_invalid_ids_and_does_not_reuse_another_session() {
        let manager = manager();
        for id in ["", "../nas", "a/b", "bad id"] {
            assert!(manager.ensure_connection(id).is_err());
        }
        assert!(manager.connection("missing").is_err());
        let draft = manager.ensure_connection("draft").unwrap();
        assert!(Arc::ptr_eq(
            &draft,
            &manager.ensure_connection("draft").unwrap()
        ));
        assert!(!Arc::ptr_eq(&draft, &manager.connection("first").unwrap()));
    }
    #[tokio::test]
    async fn lan_setting_is_restored_and_profile_saves_preserve_it() {
        let manager = manager();
        assert!(!manager.allow_lan_access.load(Ordering::Relaxed));
        set_lan_access(&manager, true).await.unwrap();
        assert!(manager.allow_lan_access.load(Ordering::Relaxed));
        let stored = storage::load_profiles(&manager.profile_path).unwrap();
        assert!(stored.allow_lan_access);
        assert_eq!(stored.profiles.len(), 2);
        let restored = AppState::new(manager.profile_path.clone(), stored);
        assert!(restored.allow_lan_access.load(Ordering::Relaxed));
        restored.persist(None, Some("first")).unwrap();
        assert!(
            storage::load_profiles(&manager.profile_path)
                .unwrap()
                .allow_lan_access
        );
        set_lan_access(&restored, false).await.unwrap();
        assert!(!restored.allow_lan_access.load(Ordering::Relaxed));
        assert!(
            !storage::load_profiles(&manager.profile_path)
                .unwrap()
                .allow_lan_access
        );
        std::fs::remove_file(&manager.profile_path).unwrap();
    }
    #[tokio::test]
    async fn lan_setting_cannot_change_while_any_connection_has_listeners() {
        let manager = manager();
        let second = manager.connection("second").unwrap();
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let route = ServiceRoute {
            id: "api".to_owned(),
            name: "API".to_owned(),
            nas_port: 8084,
            local_port: port,
            upstream: "https://api.nas-b.fnos.net/".to_owned(),
            enabled: true,
        };
        *second.proxies.lock().await = proxy::start(
            &[route],
            "nas-b",
            second.session.clone(),
            second.counter.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(set_lan_access(&manager, true).await.is_err());
        assert!(!manager.allow_lan_access.load(Ordering::Relaxed));
        assert!(!manager.profile_path.exists());
        assert_eq!(second.proxies.lock().await.len(), 1);
        stop_internal(&second).await;
        set_lan_access(&manager, true).await.unwrap();
        std::fs::remove_file(&manager.profile_path).unwrap();
    }
    #[tokio::test]
    async fn failed_lan_setting_save_does_not_change_listening_policy() {
        let mut manager = manager();
        let blocker = manager.profile_path.clone();
        std::fs::write(&blocker, "not a directory").unwrap();
        manager.profile_path = blocker.join("profile.json");
        assert!(set_lan_access(&manager, true).await.is_err());
        assert!(!manager.allow_lan_access.load(Ordering::Relaxed));
        std::fs::remove_file(blocker).unwrap();
    }
}
