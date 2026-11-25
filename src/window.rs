//! Janela do app desktop: abre a interface num navegador Chromium em "modo app"
//! (sem abas nem barra de endereço). Funciona em Windows, macOS e Linux sem
//! depender de WebView do sistema. Sem Chromium, cai no navegador padrão.

use std::path::{Path, PathBuf};
use std::process::Child;

fn candidates() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        let bases: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
            .iter()
            .filter_map(|k| std::env::var_os(k).map(PathBuf::from))
            .collect();
        for b in &bases {
            v.push(b.join("Microsoft/Edge/Application/msedge.exe"));
            v.push(b.join("Google/Chrome/Application/chrome.exe"));
            v.push(b.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
            v.push(b.join("Chromium/Application/chrome.exe"));
            v.push(b.join("Vivaldi/Application/vivaldi.exe"));
        }
    } else if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Microsoft Edge", "Brave Browser", "Chromium", "Vivaldi", "Arc"] {
            v.push(PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}")));
        }
    } else {
        let names = [
            "google-chrome-stable",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "microsoft-edge-stable",
            "brave-browser",
            "brave",
            "vivaldi",
        ];
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                for n in names {
                    v.push(dir.join(n));
                }
            }
        }
        v.push("/var/lib/flatpak/exports/bin/com.google.Chrome".into());
        v.push("/var/lib/flatpak/exports/bin/org.chromium.Chromium".into());
        v.push("/snap/bin/chromium".into());
    }
    v
}

pub fn find_chromium() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CAPIVARA_BROWSER") {
        return Some(PathBuf::from(p));
    }
    candidates().into_iter().find(|p| p.is_file())
}

/// Abre a janela. Retorna o processo do navegador (se for modo app) para o
/// app encerrar quando a janela fechar.
pub fn open(url: &str, profile: &Path) -> Option<Child> {
    if std::env::var_os("CAPIVARA_NO_APP_WINDOW").is_none() {
        if let Some(browser) = find_chromium() {
            let mut cmd = std::process::Command::new(browser);
            cmd.arg(format!("--app={url}"))
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg("--window-size=1180,860")
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-features=Translate,msEdgeSidebarV2")
                .arg("--class=Capivara");
            if let Ok(child) = cmd.spawn() {
                return Some(child);
            }
        }
    }
    let _ = open::that_detached(url);
    None
}
