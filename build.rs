use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Docker pre-builds the SPA and sets this to skip duplicate work here.
    if std::env::var("CODASAURUS_SKIP_FRONTEND_BUILD").is_ok() {
        println!("cargo:info=Skipping frontend build (CODASAURUS_SKIP_FRONTEND_BUILD=1)");
        return;
    }

    let frontend_dir = Path::new("codasaurus-frontend");
    let Some(manifest) = frontend_manifest(frontend_dir) else {
        println!(
            "cargo:warning=codasaurus-frontend/package.json not found, skipping frontend build"
        );
        return;
    };

    let node_check = Command::new("node").arg("--version").output();
    if node_check.is_err() {
        println!("cargo:warning=Node.js not found, skipping frontend build. Run 'cd codasaurus-frontend && npm install && npm run build' manually.");
        return;
    }

    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        println!("cargo:info=Installing frontend dependencies...");
        let status = Command::new("npm")
            .args(["install"])
            .current_dir(frontend_dir)
            .status()
            .expect("npm install failed");
        assert!(status.success(), "npm install failed");
    }

    println!("cargo:info=Building frontend SPA...");
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(frontend_dir)
        .status()
        .expect("npm run build failed");
    assert!(status.success(), "frontend build failed");

    println!("cargo:rerun-if-changed=codasaurus-frontend/src/");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=codasaurus-frontend/package-lock.json");

    let dist = frontend_dir.join("dist");
    assert!(dist.exists(), "frontend build did not produce dist/ output");
}

fn frontend_manifest(svelte_dir: &Path) -> Option<PathBuf> {
    let manifest = svelte_dir.join("package.json");
    manifest.is_file().then_some(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_frontend_checkout_has_no_manifest() {
        let dir = Path::new("/definitely/missing/codasaurus-frontend");
        assert!(frontend_manifest(dir).is_none());
    }
}
