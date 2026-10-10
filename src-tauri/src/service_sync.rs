use crate::text::Text;
use crate::types::{is_main_host, validate_upstream, ServiceInventory, ServiceRoute};
use std::collections::BTreeSet;

pub struct DomainSync {
    pub routes: Vec<ServiceRoute>,
    pub changed: usize,
    pub warnings: Vec<Text>,
}

// A route is identified by its NAS host port within a connection, not its domain or title.
pub fn reconcile(routes: &[ServiceRoute], inventory: &ServiceInventory, fn_id: &str) -> DomainSync {
    let mut result = DomainSync {
        routes: routes.to_vec(),
        changed: 0,
        warnings: vec![],
    };
    // A partial registry cannot prove uniqueness: a failed source could hide a second domain.
    if !["desktop", "docker"].iter().all(|id| {
        inventory
            .sources
            .iter()
            .any(|source| source.id == *id && source.status == "ok")
    }) {
        result.warnings.push(Text::new("sync.incompleteRegistry"));
        return result;
    }
    for route in &mut result.routes {
        // The NAS main endpoint is fixed, not a registry-provided port mapping.
        if validate_upstream(&route.upstream, fn_id)
            .ok()
            .is_some_and(|url| url.host_str().is_some_and(|host| is_main_host(host, fn_id)))
        {
            continue;
        }
        let candidates: BTreeSet<String> = inventory
            .services
            .iter()
            .filter(|service| service.nas_port == route.nas_port)
            .filter_map(|service| validate_upstream(&service.upstream, fn_id).ok())
            .map(|url| url.to_string())
            .collect();
        let unconfirmed = inventory
            .entries
            .iter()
            .any(|entry| entry.nas_port == Some(route.nas_port) && entry.status != "mapped");
        if unconfirmed || candidates.len() != 1 {
            let code = if unconfirmed {
                "sync.unconfirmed"
            } else if candidates.is_empty() {
                "sync.noDomain"
            } else {
                "sync.multipleDomains"
            };
            result
                .warnings
                .push(Text::with(code, [("port", route.nas_port.to_string())]));
            continue;
        }
        let address = candidates.into_iter().next().unwrap();
        if route.upstream != address {
            route.upstream = address;
            result.changed += 1;
        }
    }
    result.warnings.sort();
    result.warnings.dedup();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::error;
    use crate::inventory::{merge_inventories, parse_docker_inventory, parse_inventory};
    use serde_json::{json, Value};
    fn route(port: u16) -> ServiceRoute {
        ServiceRoute {
            id: "fixed".into(),
            name: "Custom name".into(),
            nas_port: port,
            local_port: 18084,
            upstream: "https://old-0.my-nas.fnos.net/".into(),
            enabled: false,
        }
    }
    fn inventory(desktop: Value, docker: Value) -> ServiceInventory {
        merge_inventories(
            parse_inventory(&json!({"data":{"list":desktop}}), "my-nas"),
            parse_docker_inventory(&json!({"data":{"list":docker}}), "my-nas"),
        )
        .unwrap()
    }
    #[test]
    fn main_endpoint_is_not_replaced_by_a_registered_service() {
        let report = inventory(json!([{"uri":{"port":443,"fnDomain":"app"}}]), json!([]));
        let mut main = route(443);
        for domain in ["fnos.net", "5ddd.com"] {
            main.upstream = format!("https://my-nas.{domain}/");
            let result = reconcile(&[main.clone()], &report, "my-nas");
            assert_eq!(result.routes[0].upstream, main.upstream);
            assert_eq!(result.changed, 0);
            assert!(result.warnings.is_empty());
        }
    }
    #[test]
    fn refreshes_by_nas_port_and_preserves_all_other_settings() {
        let report = inventory(
            json!([
                {"title":"Renamed", "uri":{"port":8084,"fnDomain":"new-0"}},
                {"title":"Custom name", "uri":{"port":9999,"fnDomain":"wrong-0"}}
            ]),
            json!([]),
        );
        let result = reconcile(&[route(8084)], &report, "my-nas");
        assert_eq!(result.changed, 1);
        assert_eq!(result.routes[0].upstream, "https://new-0.my-nas.fnos.net/");
        assert_eq!(result.routes[0].id, "fixed");
        assert_eq!(result.routes[0].name, "Custom name");
        assert_eq!(result.routes[0].nas_port, 8084);
        assert_eq!(result.routes[0].local_port, 18084);
        assert!(!result.routes[0].enabled);
        assert!(result.warnings.is_empty());
        assert_eq!(reconcile(&result.routes, &report, "my-nas").changed, 0);
    }
    #[test]
    fn duplicate_same_domain_across_sources_is_not_ambiguous() {
        let report = inventory(
            json!([{"uri":{"port":"8084","fnDomain":"new-0"}}]),
            json!([{"appID":"container", "uri":{"port":8084,"fnDomain":"new-0"}}]),
        );
        assert_eq!(reconcile(&[route(8084)], &report, "my-nas").changed, 1);
    }
    #[test]
    fn ambiguous_missing_unconfirmed_and_unsafe_ports_keep_the_cache() {
        for report in [
            inventory(
                json!([{"uri":{"port":8084,"fnDomain":"new-0"}}]),
                json!([{"uri":{"port":8084,"fnDomain":"other-0"}}]),
            ),
            inventory(json!([]), json!([])),
            inventory(
                json!([{"uri":{"port":8084,"fnDomain":"new-0"}},
                {"uri":{"port":8084}}]),
                json!([]),
            ),
            inventory(
                json!([{"uri":{"port":8084,"fnDomain":"evil.test/"}}]),
                json!([]),
            ),
        ] {
            let result = reconcile(&[route(8084)], &report, "my-nas");
            assert_eq!(result.changed, 0);
            assert_eq!(result.routes[0].upstream, route(8084).upstream);
            assert!(!result.warnings.is_empty());
        }
    }
    #[test]
    fn incomplete_sources_never_confirm_uniqueness() {
        let report = merge_inventories(
            parse_inventory(
                &json!({"data":{"list":[{"uri":{"port":8084,"fnDomain":"new-0"}}]}}),
                "my-nas",
            ),
            Err(error("denied")),
        )
        .unwrap();
        assert_eq!(reconcile(&[route(8084)], &report, "my-nas").changed, 0);
        let report = merge_inventories(
            Err(error("denied")),
            parse_docker_inventory(
                &json!({"data":{"list":[{"uri":{"port":8084,"fnDomain":"new-0"}}]}}),
                "my-nas",
            ),
        )
        .unwrap();
        assert_eq!(reconcile(&[route(8084)], &report, "my-nas").changed, 0);
    }
    #[test]
    fn another_nas_cannot_supply_an_upstream_for_this_connection() {
        let report = inventory(json!([{"uri":{"port":8084,"fnDomain":"new-0"}}]), json!([]));
        assert_eq!(reconcile(&[route(8084)], &report, "other-nas").changed, 0);
    }
}
