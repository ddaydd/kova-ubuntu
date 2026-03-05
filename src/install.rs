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

fn nemo_action(bin: &std::path::Path) -> String {
    format!(
        r#"[Nemo Action]
Name=Open in Kova
Comment=Open a terminal here
Exec={bin} %F
Icon-Name=utilities-terminal
Selection=none
Extensions=any;
"#,
        bin = bin.display()
    )
}

fn desktop_path() -> PathBuf {
    PathBuf::from(home()).join(".local/share/applications/kova.desktop")
}

fn nemo_action_path() -> PathBuf {
    PathBuf::from(home()).join(".local/share/nemo/actions/kova.nemo_action")
}

fn autostart_path() -> PathBuf {
    PathBuf::from(home()).join(".config/autostart/kova.desktop")
}

fn local_bin_path() -> PathBuf {
    PathBuf::from(home()).join(".local/bin/kova")
}

pub fn install(autostart: bool) {
    // Symlink binary to ~/.local/bin/kova
    let bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/kova"));
    let link = local_bin_path();
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    // Remove existing symlink/file before creating
    let _ = fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&bin, &link).unwrap();
    println!("Symlinked {} -> {}", link.display(), bin.display());

    let desktop = desktop_path();
    let dir = desktop.parent().unwrap();
    fs::create_dir_all(dir).unwrap();
    fs::write(&desktop, desktop_entry()).unwrap();
    println!("Installed {}", desktop.display());

    // Update desktop database
    let _ = std::process::Command::new("update-desktop-database")
        .arg(dir)
        .status();

    // Nemo action for right-click "Open in Kova"
    let nemo_action_file = nemo_action_path();
    let nemo_dir = nemo_action_file.parent().unwrap();
    fs::create_dir_all(nemo_dir).unwrap();
    let bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/kova"));
    fs::write(&nemo_action_file, nemo_action(&bin)).unwrap();
    println!("Installed {}", nemo_action_file.display());

    if autostart {
        let autostart_file = autostart_path();
        fs::create_dir_all(autostart_file.parent().unwrap()).unwrap();
        fs::write(&autostart_file, desktop_entry()).unwrap();
        println!("Installed {}", autostart_file.display());
        println!("Kova will start automatically at login.");
    }

    println!("Done! 'kova' is now available globally, in your application menu, and in 'Open With' for folders.");
    if !autostart {
        println!("To also start Kova at login, run: kova --install --autostart");
    }
}

pub fn uninstall() {
    let link = local_bin_path();
    if link.exists() || link.symlink_metadata().is_ok() {
        fs::remove_file(&link).unwrap();
        println!("Removed {}", link.display());
    }

    let desktop = desktop_path();
    if desktop.exists() {
        fs::remove_file(&desktop).unwrap();
        println!("Removed {}", desktop.display());
        let _ = std::process::Command::new("update-desktop-database")
            .arg(desktop.parent().unwrap())
            .status();
    }

    let nemo = nemo_action_path();
    if nemo.exists() {
        fs::remove_file(&nemo).unwrap();
        println!("Removed {}", nemo.display());
    }

    let autostart = autostart_path();
    if autostart.exists() {
        fs::remove_file(&autostart).unwrap();
        println!("Removed {}", autostart.display());
    }

    println!("Done! Kova desktop integration removed.");
}
