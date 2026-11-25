// Embute ícone e metadados no capivara.exe (só no Windows).
fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/capivara.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/capivara.ico")
            .set("ProductName", "Capivara")
            .set("FileDescription", "Capivara - video, audio e transcricao")
            .set("CompanyName", "Capivara")
            .set("LegalCopyright", "MIT");
        if let Err(e) = res.compile() {
            // Sem compilador de recursos o app funciona igual, só fica sem ícone.
            println!("cargo:warning=ícone não embutido: {e}");
        }
    }
}
