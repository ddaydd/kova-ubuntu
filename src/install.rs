use std::fs;
use std::path::PathBuf;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".into())
}

fn desktop_entry() -> String {
    let bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/kova"));
    format!(
        r#"[Desktop Entry]
Name=Kova
Comment=Fast GPU-accelerated terminal
Exec={bin} %u
Icon=utilities-terminal
Type=Application
Terminal=false
Categories=System;TerminalEmulator;
MimeType=inode/directory;
Actions=open-here;

[Desktop Action open-here]
Name=Open Terminal Here
Exec={bin}
"#,
        bin = bin.display()
    )
}

fn desktop_path() -> PathBuf {
    PathBuf::from(home()).join(".local/share/applications/kova.desktop")
}

fn autostart_path() -> PathBuf {
    PathBuf::from(home()).join(".config/autostart/kova.desktop")
}

pub fn install(autostart: bool) {
    let desktop = desktop_path();
    let dir = desktop.parent().unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(&desktop, desktop_entry()).unwrap();
    println!("Installed {}", desktop.display());

    // Update desktop database
    let _ = std::process::Command::new("update-desktop-database")
        .arg(dir)
        .status();

    if autostart {
        let autostart_file = autostart_path();
        fs::create_dir_all(autostart_file.parent().unwrap()).unwrap();
        fs::write(&autostart_file, desktop_entry()).unwrap();
        println!("Installed {}", autostart_file.display());
        println!("Kova will start automatically at login.");
    }

    println!("Done! Kova is now available in your application menu and 'Open With' for folders.");
    if !autostart {
        println!("To also start Kova at login, run: kova --install --autostart");
    }
}

pub fn uninstall() {
    let desktop = desktop_path();
    if desktop.exists() {
        fs::remove_file(&desktop).unwrap();
        println!("Removed {}", desktop.display());
        let _ = std::process::Command::new("update-desktop-database")
            .arg(desktop.parent().unwrap())
            .status();
    }

    let autostart = autostart_path();
    if autostart.exists() {
        fs::remove_file(&autostart).unwrap();
        println!("Removed {}", autostart.display());
    }

    println!("Done! Kova desktop integration removed.");
}
