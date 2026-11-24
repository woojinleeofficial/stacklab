//! UI strings from the resource file (spec 16: Korean UI, strings separated from code).

use std::collections::BTreeMap;

const KO: &str = include_str!("../i18n/ko.toml");

pub struct Strings {
    map: BTreeMap<String, String>,
}

impl Strings {
    pub fn korean() -> Strings {
        let map: BTreeMap<String, String> = toml::from_str(KO).expect("ko.toml parses");
        Strings { map }
    }

    pub fn get(&self, key: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[{key}]"))
    }

    /// `get` with `{name}` placeholders replaced.
    pub fn fmt(&self, key: &str, args: &[(&str, String)]) -> String {
        let mut s = self.get(key);
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_used_in_code_exists() {
        let s = Strings::korean();
        let code = [include_str!("main.rs"), include_str!("draw.rs")].concat();
        let mut missing = Vec::new();
        for part in code
            .split("s.get(\"")
            .skip(1)
            .chain(code.split("s.fmt(\"").skip(1))
        {
            let key = part.split('"').next().unwrap_or_default();
            if !s.map.contains_key(key) {
                missing.push(key.to_string());
            }
        }
        assert!(missing.is_empty(), "missing strings: {missing:?}");
    }
}
