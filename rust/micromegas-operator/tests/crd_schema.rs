use kube::CustomResourceExt;
use micromegas_operator::crds::{MicromegasInstance, Screen};

fn schema_of<K: CustomResourceExt>() -> serde_json::Value {
    let crd = serde_json::to_value(K::crd()).unwrap();
    crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"].clone()
}

#[test]
fn screen_crd_identity() {
    let crd = Screen::crd();
    assert_eq!(crd.spec.group, "micromegas.info");
    assert_eq!(crd.spec.names.kind, "Screen");
    assert_eq!(crd.spec.names.plural, "screens");
    assert_eq!(crd.spec.scope, "Namespaced");
    assert!(
        crd.spec.versions[0]
            .subresources
            .as_ref()
            .unwrap()
            .status
            .is_some()
    );
}

#[test]
fn screen_config_preserves_unknown_fields() {
    let schema = schema_of::<Screen>();
    let config = &schema["properties"]["spec"]["properties"]["config"];
    assert_eq!(config["x-kubernetes-preserve-unknown-fields"], true);
}

fn rules(node: &serde_json::Value) -> Vec<String> {
    node["x-kubernetes-validations"]
        .as_array()
        .expect("validation rules")
        .iter()
        .map(|r| r["rule"].as_str().expect("rule text").to_string())
        .collect()
}

#[test]
fn screen_spec_has_one_of_rule_and_immutability_rules() {
    let schema = schema_of::<Screen>();
    let spec = &schema["properties"]["spec"];
    let spec_rules = rules(spec);
    assert!(spec_rules.iter().any(|r| r.contains("has(self.config)")));
    // Guards unset -> set on the optional name, which `self == oldSelf` skips.
    assert!(spec_rules.iter().any(|r| r.contains("has(oldSelf.name)")));
    assert!(
        rules(&spec["properties"]["name"])
            .iter()
            .any(|r| r == "self == oldSelf")
    );
    assert_eq!(
        rules(&spec["properties"]["screenType"]),
        ["self == oldSelf"]
    );
}

#[test]
fn screen_spec_has_admission_patterns() {
    let schema = schema_of::<Screen>();
    let spec = &schema["properties"]["spec"];
    assert!(
        rules(&spec["properties"]["name"])
            .iter()
            .any(|r| r.contains("^[a-z][a-z0-9-]{1,98}[a-z0-9]$"))
    );
    assert!(
        rules(&spec["properties"]["folderPath"])
            .iter()
            .any(|r| r.contains("^[a-z0-9-]+(/[a-z0-9-]+)*$"))
    );
}

#[test]
fn screen_defaults_apply_on_deserialize() {
    let yaml = r#"
apiVersion: micromegas.info/v1alpha1
kind: Screen
metadata: { name: demo, namespace: default }
spec:
  instanceSelector: { matchLabels: { env: dev } }
  config: { cells: [] }
"#;
    let screen: Screen = serde_norway::from_str(yaml).unwrap();
    assert_eq!(screen.spec.screen_type, "notebook");
    assert_eq!(screen.spec.folder_path, "");
    assert!(screen.spec.name.is_none());
}

#[test]
fn instance_crd_identity_and_defaults() {
    let crd = MicromegasInstance::crd();
    assert_eq!(crd.spec.names.plural, "micromegasinstances");
    assert_eq!(crd.spec.names.short_names, Some(vec!["mmi".to_string()]));
    let yaml = r#"
apiVersion: micromegas.info/v1alpha1
kind: MicromegasInstance
metadata: { name: dev, namespace: default }
spec: { url: "http://127.0.0.1:3000" }
"#;
    let inst: MicromegasInstance = serde_norway::from_str(yaml).unwrap();
    assert_eq!(inst.spec.resync_interval, "10m");
    assert!(inst.spec.auth.is_none());
}
