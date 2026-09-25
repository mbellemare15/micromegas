//! Writes the CRD manifests the Helm chart installs, or verifies they are current.

use clap::Parser;
use kube::CustomResourceExt;
use micromegas_operator::crds::{MicromegasInstance, Screen};
use std::path::PathBuf;

#[derive(Parser)]
#[clap(about = "Generate CRD YAML for charts/micromegas-operator/crds")]
struct Cli {
    /// Directory receiving one file per CRD.
    out_dir: PathBuf,
    /// Exit non-zero if the files on disk differ from the generated output.
    #[clap(long)]
    check: bool,
}

fn render() -> Vec<(&'static str, String)> {
    vec![
        (
            "micromegasinstances.micromegas.info.yaml",
            serde_norway::to_string(&MicromegasInstance::crd()).expect("serialize CRD"),
        ),
        (
            "screens.micromegas.info.yaml",
            serde_norway::to_string(&Screen::crd()).expect("serialize CRD"),
        ),
    ]
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut stale = Vec::new();
    for (file, content) in render() {
        let path = cli.out_dir.join(file);
        if cli.check {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            if on_disk != content {
                stale.push(path.display().to_string());
            }
        } else {
            std::fs::create_dir_all(&cli.out_dir)?;
            std::fs::write(&path, content)?;
            println!("wrote {}", path.display());
        }
    }
    if !stale.is_empty() {
        anyhow::bail!(
            "CRD files are stale: {}. Run: cargo run -p micromegas-operator --bin crdgen -- ../charts/micromegas-operator/crds",
            stale.join(", ")
        );
    }
    Ok(())
}
