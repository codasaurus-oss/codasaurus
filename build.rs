use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Docker pre-builds the SPA and sets this to skip duplicate work here.
    if std::env::var("CODASAURUS_SKIP_FRONTEND_BUILD").is_ok() {
        println!("cargo:info=Skipping Svelte build (CODASAURUS_SKIP_FRONTEND_BUILD=1)");
        return;
    }

    let svelte_dir = Path::new("svelte-dashboard");
    let Some(manifest) = frontend_manifest(svelte_dir) else {
        println!("cargo:warning=svelte-dashboard/package.json not found, skipping frontend build");
        return;
    };

    let node_check = Command::new("node").arg("--version").output();
    if node_check.is_err() {
        println!("cargo:warning=Node.js not found, skipping Svelte build. Run 'cd svelte-dashboard && npm install && npm run build' manually.");
        return;
    }

    let node_modules = svelte_dir.join("node_modules");
    if !node_modules.exists() {
        println!("cargo:info=Installing Svelte dependencies...");
        let status = Command::new("npm")
            .args(["install"])
            .current_dir(svelte_dir)
            .status()
            .expect("npm install failed");
        assert!(status.success(), "npm install failed");
    }

    println!("cargo:info=Building Svelte SPA...");
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(svelte_dir)
        .status()
        .expect("npm run build failed");
    assert!(status.success(), "Svelte build failed");

    println!("cargo:rerun-if-changed=svelte-dashboard/src/");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=svelte-dashboard/package-lock.json");

    let dist = svelte_dir.join("dist");
    assert!(dist.exists(), "Svelte build did not produce dist/ output");
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
        let dir = Path::new("/definitely/missing/svelte-dashboard");
        assert!(frontend_manifest(dir).is_none());
    }
}
