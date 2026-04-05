use serde::Deserialize;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct UuidEntry {
    uuid: String,
    name: String,
    #[allow(unused)]
    id: String,
}

#[derive(Debug, Deserialize)]
struct UuidDatabase {
    uuids: Vec<UuidEntry>,
}

fn main() {
    // Re-run if the YAML file changes
    println!("cargo:rerun-if-changed=data/service_uuids.yaml");
    println!("cargo:rerun-if-changed=data/characteristic_uuids.yaml");

    let mut map = String::new();

    // Generate BLE service map
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("uuid_map.rs");

    let service_yaml =
        fs::read_to_string("data/service_uuids.yaml").expect("Failed to read service_uuids.yaml");
    let service_db: UuidDatabase =
        serde_yaml::from_str(&service_yaml).expect("Failed to parse YAML");

    map.push_str(
        r#"
static SERVICE_MAP: std::sync::LazyLock<std::collections::HashMap<uuid::Uuid,&'static str>> =
    std::sync::LazyLock::new(|| {
        std::collections::HashMap::from([
"#,
    );
    for entry in service_db.uuids.iter() {
        map.push_str(&format!(
            "            (parse_uuid(\"{}\").unwrap(),\"{}\"),\n",
            entry.uuid, entry.name
        ));
    }
    map.push_str(
        r#"        ])
    });
"#,
    );

    // Generate BLE characteristics map
    let characteristic_yaml = fs::read_to_string("data/characteristic_uuids.yaml")
        .expect("Failed to read characteristic_uuids.yaml");
    let characteristic_db: UuidDatabase =
        serde_yaml::from_str(&characteristic_yaml).expect("Failed to parse YAML");

    map.push_str(
        r#"
static CHARACTERISTIC_MAP: std::sync::LazyLock<std::collections::HashMap<uuid::Uuid,&'static str>> =
    std::sync::LazyLock::new(|| {
        std::collections::HashMap::from([
"#,
    );
    for entry in characteristic_db.uuids.iter() {
        map.push_str(&format!(
            "            (parse_uuid(\"{}\").unwrap(),\"{}\"),\n",
            entry.uuid, entry.name
        ));
    }
    map.push_str(
        r#"        ])
    });
"#,
    );

    fs::write(&dest_path, map).expect("Failed to write generated uuid_map file");
}
