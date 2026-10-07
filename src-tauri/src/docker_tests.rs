use super::*;
use crate::inventory::{
    merge_inventories, parse_docker_inventory, parse_inventory, unavailable_registries,
};
use serde_json::json;

fn packet(value: Value) -> ContainerPacket {
    serde_json::from_value(value).unwrap()
}
fn container(id: &str, ports: Value) -> ContainerMetadata {
    serde_json::from_value(
        json!({"id":id,"names":["/fixture service"],"state":"running","ports":ports}),
    )
    .unwrap()
}
fn registries(records: Value) -> ServiceInventory {
    merge_inventories(
        parse_inventory(&json!({"data":{"list":[]}}), "my-nas"),
        parse_docker_inventory(&json!({"data":{"list":records}}), "my-nas"),
    )
    .unwrap()
}
#[test]
fn stream_requires_terminal_and_unions_duplicate_container_ports() {
    let mut collector = ContainerCollector::new(StreamLimits::default());
    assert!(!collector
        .push(packet(
            json!({"result":"doing","rsp":[{"id":"fixture-a","ports":[
                {"publicPort":8084,"privatePort":80,"type":"tcp"},
                {"publicPort":8084,"privatePort":80,"type":"tcp"}
            ]}]})
        ))
        .unwrap());
    assert!(!collector
        .push(packet(
            json!({"result":"doing","rsp":[{"id":"fixture-a","names":["/latest name"],"ports":[
                {"publicPort":3000,"privatePort":3000,"type":"TCP"}
            ]}]})
        ))
        .unwrap());
    assert!(collector.push(packet(json!({"result":"succ"}))).unwrap());
    let items = collector.finish().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].ports.len(), 2);
    assert_eq!(items[0].ports[1].protocol, "tcp");
    assert_eq!(items[0].names, vec!["/latest name"]);
    let mut incomplete = ContainerCollector::new(StreamLimits::default());
    incomplete
        .push(packet(json!({"result":"doing","rsp":[{"id":"fixture-a"}]})))
        .unwrap();
    assert!(incomplete.finish().is_err());
}
#[test]
fn partial_failure_unknown_status_and_limits_never_produce_inventory() {
    let mut collector = ContainerCollector::new(StreamLimits::default());
    collector
        .push(packet(json!({"result":"doing","rsp":[{"id":"fixture-a"}]})))
        .unwrap();
    assert!(collector
        .push(packet(json!({"result":"fail","errno":9999})))
        .is_err());
    assert!(collector.finish().is_err());
    let mut collector = ContainerCollector::new(StreamLimits::default());
    assert!(collector
        .push(packet(json!({"result":"something-new"})))
        .is_err());
    let mut collector = ContainerCollector::new(StreamLimits {
        records: 1,
        ..StreamLimits::default()
    });
    assert!(collector
        .push(packet(
            json!({"result":"succ","rsp":[{"id":"a"},{"id":"b"}]})
        ))
        .is_err());
    assert!(collector.finish().is_err());
    let mut collector = ContainerCollector::new(StreamLimits {
        bindings: 0,
        ..StreamLimits::default()
    });
    assert!(collector
        .push(packet(
            json!({"result":"succ","rsp":[{"id":"a","ports":[{"publicPort":80,"type":"tcp"}]}]})
        ))
        .is_err());
    assert!(collector.finish().is_err());
}
#[test]
fn join_uses_container_prefix_and_host_port_not_private_or_global_port() {
    let mut report = registries(json!([
        {"appID":"container-a","uri":{"port":"8084","fnDomain":"a-0"}},
        {"appID":"container-b","uri":{"port":"9090","fnDomain":"b-0"}}
    ]));
    attach_container_ports(
        &mut report,
        Ok(vec![
            container(
                "container-a-full",
                json!([
                    {"publicPort":8084,"privatePort":80,"type":"tcp"},
                    {"publicPort":80,"privatePort":8084,"type":"tcp"}
                ]),
            ),
            container(
                "container-b-full",
                json!([{ "publicPort":8084,"privatePort":80,"type":"tcp"}]),
            ),
        ]),
    );
    let docker = report.docker.unwrap();
    assert_eq!(docker.rows[0].status, "mapped");
    assert_eq!(docker.rows[0].fn_domain.as_deref(), Some("a-0"));
    assert_eq!(docker.rows[1].status, "no-domain");
    assert_eq!(docker.rows[2].status, "no-domain");
    assert_eq!(docker.mapped_ports, 1);
    assert_eq!(docker.unmapped_ports, 2);
}
#[test]
fn udp_exposed_only_and_invalid_ports_are_never_proxy_targets() {
    let mut report =
        registries(json!([{ "appID":"fixture-a","uri":{"port":8084,"fnDomain":"a-0"} }]));
    attach_container_ports(
        &mut report,
        Ok(vec![container(
            "fixture-a-full",
            json!([
                {"publicPort":8084,"privatePort":80,"type":"udp"},
                {"privatePort":5432,"type":"tcp"},
                {"publicPort":65536,"privatePort":80,"type":"tcp"},
                {"publicPort":true,"privatePort":80,"type":"tcp"},
                {"publicPort":"8084","privatePort":"80","type":"tcp"}
            ]),
        )]),
    );
    let docker = report.docker.unwrap();
    assert_eq!(docker.published_ports, 2);
    assert_eq!(docker.rows[0].status, "unsupported-protocol");
    assert_eq!(docker.rows[1].status, "not-published");
    assert_eq!(docker.rows[2].status, "not-published");
    assert_eq!(docker.rows[3].status, "not-published");
    assert_eq!(docker.rows[4].status, "mapped");
    assert!(docker.rows[..4].iter().all(|row| row.upstream.is_none()));
}
#[test]
fn registry_failure_is_unknown_and_failed_stream_does_not_show_zero_ports() {
    let mut report = unavailable_registries();
    attach_container_ports(
        &mut report,
        Ok(vec![container(
            "fixture-a",
            json!([{ "publicPort":80,"type":"tcp"}]),
        )]),
    );
    let docker = report.docker.unwrap();
    assert!(!docker.registry_available);
    assert_eq!(docker.unmapped_ports, 0);
    assert_eq!(docker.unconfirmed_ports, 1);
    assert_eq!(docker.rows[0].status, "registry-unavailable");
    let mut report = unavailable_registries();
    attach_container_ports(&mut report, Err(error("fixture interrupted")));
    assert!(report.docker.is_none());
    assert_eq!(report.sources[2].status, "unavailable");
    assert_eq!(report.sources[2].count, None);
}
#[test]
fn overlapping_container_prefixes_or_multiple_domains_are_ambiguous() {
    let mut report = registries(json!([{ "appID":"fixture-","uri":{"port":80,"fnDomain":"a-0"} }]));
    attach_container_ports(
        &mut report,
        Ok(vec![
            container("fixture-a", json!([{ "publicPort":80,"type":"tcp"}])),
            container("fixture-b", json!([{ "publicPort":80,"type":"tcp"}])),
        ]),
    );
    assert!(report
        .docker
        .unwrap()
        .rows
        .iter()
        .all(|row| row.status == "ambiguous" && row.upstream.is_none()));
    let mut report = registries(json!([
        {"appID":"fixture-a","uri":{"port":80,"fnDomain":"a-0"}},
        {"appID":"fixture-a","uri":{"port":80,"fnDomain":"a-1"}}
    ]));
    attach_container_ports(
        &mut report,
        Ok(vec![container(
            "fixture-a-full",
            json!([{ "publicPort":80,"type":"tcp"}]),
        )]),
    );
    assert_eq!(report.docker.unwrap().rows[0].status, "ambiguous");
}
#[test]
fn unrequested_sensitive_fields_are_not_retained_or_forwarded() {
    let item: ContainerMetadata = serde_json::from_value(json!({
        "id":"fixture-a","names":["/fixture"],"state":"exited",
        "env":["API_KEY=fixture-secret-never-forward"],"hostConfig":{"password":"fixture-secret-never-forward"},
        "ports":[{"publicPort":8084,"privatePort":80,"type":"tcp"}]
    })).unwrap();
    let mut report =
        registries(json!([{ "appID":"fixture-a","uri":{"port":8084,"fnDomain":"a-0"} }]));
    attach_container_ports(&mut report, Ok(vec![item]));
    let text = serde_json::to_string(&report).unwrap();
    assert!(!text.contains("fixture-secret-never-forward"));
    assert!(!text.contains("hostConfig"));
    assert!(!text.contains("env"));
    assert_eq!(report.docker.unwrap().rows[0].state, "exited");
}

#[test]
fn null_or_missing_port_lists_are_empty_but_wrong_shapes_are_rejected() {
    let empty: ContainerMetadata =
        serde_json::from_value(json!({"id":"fixture-a", "names":null, "ports":null})).unwrap();
    assert!(empty.names.is_empty());
    assert!(empty.ports.is_empty());
    assert!(serde_json::from_value::<ContainerMetadata>(
        json!({"id":"fixture-a","ports":{"unexpected":"schema"}})
    )
    .is_err());
}
