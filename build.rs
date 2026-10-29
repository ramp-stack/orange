use std::{env, fs, io, path::PathBuf};

fn main() {
    let path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join(".env");
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-env-changed=GIPHY_API_KEY");

    let key = env::var("GIPHY_API_KEY").ok().or_else(|| {
        match fs::read_to_string(path) {
            Ok(contents) => giphy_key(&contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(_) => panic!("Cannot read Orange's .env file"),
        }
    });
    if let Some(key) = key {
        // A GIPHY key is a literal token; reject line breaks and other syntax
        // rather than allowing .env contents to become Cargo directives.
        assert!(key.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "GIPHY_API_KEY must contain only letters and digits");
        println!("cargo:rustc-env=GIPHY_API_KEY={key}");
    }
}

fn giphy_key(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let (name, value) = line.split_once('=')?;
        if name.trim() != "GIPHY_API_KEY" {return None;}
        let value = value.split('#').next().unwrap().trim();
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        Some(value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::giphy_key;

    #[test]
    fn reads_only_the_giphy_token() {
        for line in ["GIPHY_API_KEY=example123", "export GIPHY_API_KEY = \"example123\" # GIFs", "GIPHY_API_KEY='example123'\r\n"] {
            assert_eq!(giphy_key(line).as_deref(), Some("example123"));
        }
        assert_eq!(giphy_key("# GIPHY_API_KEY=ignored\nOTHER_KEY=ignored"), None);
    }
}
