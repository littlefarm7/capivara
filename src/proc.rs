//! Helpers para rodar processos externos de forma silenciosa e portátil.

use std::ffi::OsStr;
use tokio::process::Command;

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    c.env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: não pisca janela de console no Windows.
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Roda e devolve stdout (erro com stderr se falhar).
pub async fn output(program: impl AsRef<OsStr>, args: &[&str]) -> anyhow::Result<String> {
    let out = command(program).args(args).output().await?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
