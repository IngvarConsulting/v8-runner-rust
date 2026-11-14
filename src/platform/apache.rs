use std::path::{Path, PathBuf};

use crate::platform::process::ProcessRequest;
use crate::platform::service::ServiceRequest;

pub fn locate(hint: Option<&Path>) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["httpd.exe"]
    } else {
        &["apache2", "httpd"]
    };
    let mut candidates = Vec::new();
    if let Some(path) = hint {
        candidates.push(path.to_path_buf());
        for name in names {
            candidates.extend([path.join(name), path.join("bin").join(name)]);
        }
    } else {
        let installed: &[&str] = if cfg!(windows) {
            &[
                "C:/Apache24/bin",
                "C:/Program Files/Apache Group/Apache24/bin",
            ]
        } else {
            &["/usr/sbin", "/usr/local/apache2/bin"]
        };
        for directory in std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .chain(installed.iter().map(PathBuf::from))
        {
            candidates.extend(names.iter().map(|name| directory.join(name)));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn quoted(path: &Path) -> std::io::Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| std::io::Error::other("Apache paths must be UTF-8"))?;
    if path.contains(['"', '\n', '\r']) {
        return Err(std::io::Error::other("invalid character in an Apache path"));
    }
    Ok(format!("\"{}\"", path.replace('\\', "/")))
}

pub fn platform_module(ragent: &Path) -> PathBuf {
    ragent.with_file_name(if cfg!(windows) {
        "wsap24.dll"
    } else {
        "wsap24.so"
    })
}

pub fn module_directive(module: &Path) -> std::io::Result<String> {
    Ok(format!("LoadModule _1cws_module {}", quoted(module)?))
}

pub fn config(program: &Path, module: &Path, root: &Path, port: u16) -> std::io::Result<String> {
    let bin = program.parent().unwrap_or(program);
    let install = bin.parent().unwrap_or(bin);
    let modules = if cfg!(windows) {
        install.join("modules")
    } else {
        [
            install.join("modules"),
            install.join("lib/apache2/modules"),
            install.join("lib64/httpd/modules"),
        ]
        .into_iter()
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| install.join("lib/httpd/modules"))
    };
    let mut result = format!(
        "ServerRoot {}\nServerName localhost\nListen 127.0.0.1:{port}\nPidFile {}\nErrorLog {}\nDocumentRoot {}\n",
        quoted(install)?, quoted(&root.join("httpd.pid"))?,
        quoted(&root.join("error.log"))?, quoted(&root.join("htdocs"))?,
    );
    for (name, file) in [
        ("mpm_prefork_module", "mod_mpm_prefork.so"),
        ("authz_core_module", "mod_authz_core.so"),
        ("alias_module", "mod_alias.so"),
    ] {
        if name.starts_with("mpm_") && cfg!(windows) {
            continue;
        }
        let path = modules.join(file);
        if path.is_file() {
            result.push_str(&format!("LoadModule {name} {}\n", quoted(&path)?));
        }
    }
    result.push_str(&module_directive(module)?);
    result.push('\n');
    Ok(result)
}

/// Update the generated directives. Other directives may have been added by webinst.
pub fn reconfigure(previous: &str, rendered: &str) -> std::io::Result<String> {
    const LISTEN: &str = "Listen 127.0.0.1:";
    const PID_FILE: &str = "PidFile ";
    const MODULE: &str = "LoadModule _1cws_module ";
    let expected_pid = directive(rendered, PID_FILE)?;
    if directive(previous, PID_FILE)? != expected_pid {
        return Err(std::io::Error::other(
            "Apache config belongs to a different stand path",
        ));
    }
    let old_listen = directive(previous, LISTEN)?;
    let old_module = directive(previous, MODULE)?;
    let new_listen = directive(rendered, LISTEN)?;
    let new_module = directive(rendered, MODULE)?;
    Ok(previous
        .replacen(old_module, new_module, 1)
        .replacen(old_listen, new_listen, 1))
}

fn directive<'a>(config: &'a str, prefix: &str) -> std::io::Result<&'a str> {
    let mut matching = config.lines().filter(|line| line.starts_with(prefix));
    matching
        .next()
        .filter(|_| matching.next().is_none())
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "private Apache config requires one {prefix} directive"
            ))
        })
}

pub fn foreground(program: &Path, conf: &Path, log: &Path) -> ServiceRequest {
    let args = if cfg!(windows) {
        vec!["-f".to_owned(), conf.display().to_string()]
    } else {
        vec![
            "-DFOREGROUND".to_owned(),
            "-f".to_owned(),
            conf.display().to_string(),
        ]
    };
    ServiceRequest {
        program: program.to_path_buf(),
        args,
        workdir: conf.parent().map(Path::to_path_buf),
        log_path: log.to_path_buf(),
        stdin: None,
    }
}

pub fn check(program: &Path, conf: &Path) -> ProcessRequest {
    ProcessRequest {
        program: program.to_path_buf(),
        args: vec!["-t".to_owned(), "-f".to_owned(), conf.display().to_string()],
        workdir: conf.parent().map(Path::to_path_buf),
        stdout_log_path: None,
        stderr_log_path: None,
        startup_probe: None,
        windows_verbatim_args: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_configuration_names_only_its_loopback_port_and_files() {
        let config = config(
            Path::new("install/bin/httpd"),
            Path::new("platform/wsap24"),
            Path::new("stand/web"),
            24700,
        )
        .unwrap();
        assert!(config.lines().any(|line| line == "Listen 127.0.0.1:24700"));
        assert!(config.contains("PidFile \"stand/web/httpd.pid\""));
        assert!(config.contains("ErrorLog \"stand/web/error.log\""));
        assert!(config.contains("LoadModule _1cws_module \"platform/wsap24\""));
    }
}
