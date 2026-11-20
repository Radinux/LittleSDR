//! Country of a callsign from the CTY database (cty.dat, AD1C's format, shipped in the
//! app's res/cty): longest matching prefix, or the exact call ("=CALL"). Gives the country
//! name and its position.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Country {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Default)]
pub struct Cty {
    countries: Vec<Country>,
    prefixes: HashMap<String, usize>,
    exact: HashMap<String, usize>,
}

static CTY: OnceLock<Cty> = OnceLock::new();

/// Loads the database once (later calls keep the first)
pub fn load(path: &str) {
    let _ = CTY.get_or_init(|| std::fs::read_to_string(path).map(|t| Cty::parse(&t)).unwrap_or_default());
}

pub fn lookup(call: &str) -> Option<(f64, f64)> {
    CTY.get()?.find(call).map(|c| (c.lat, c.lon))
}

pub fn country(call: &str) -> Option<String> {
    CTY.get()?.find(call).map(|c| c.name.clone())
}

impl Cty {
    pub fn parse(text: &str) -> Cty {
        let mut cty = Cty::default();
        let mut cur: Option<usize> = None;
        for line in text.lines() {
            if !line.starts_with(char::is_whitespace) && line.contains(':') {
                let f: Vec<&str> = line.split(':').map(str::trim).collect();
                if f.len() >= 8 {
                    let (lat, lon) = (f[4].parse().unwrap_or(0.0), f[5].parse::<f64>().unwrap_or(0.0));
                    cty.countries.push(Country { name: f[0].to_string(), lat, lon: -lon }); // file: west positive
                    let i = cty.countries.len() - 1;
                    cur = Some(i);
                    cty.prefixes.insert(f[7].trim_start_matches('*').to_uppercase(), i);
                }
                continue;
            }
            let Some(i) = cur else { continue };
            for item in line.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
                // Drop the overrides: (CQ) [ITU] <lat/lon> {continent} ~tz~
                let p: String = item.chars().take_while(|c| !"([<{~".contains(*c)).collect::<String>().to_uppercase();
                if let Some(c) = p.strip_prefix('=') {
                    cty.exact.insert(c.to_string(), i);
                } else if !p.is_empty() {
                    cty.prefixes.insert(p, i);
                }
            }
        }
        cty
    }

    pub fn find(&self, call: &str) -> Option<&Country> {
        let c = call.trim().trim_matches(|c| c == '<' || c == '>').to_uppercase();
        if let Some(&i) = self.exact.get(&c) {
            return self.countries.get(i);
        }
        // "EA8/F4ABC": the prefix part names the country when it is the shorter one
        let base = match c.split_once('/') {
            Some((a, b)) if a.len() <= b.len() && !a.is_empty() => a.to_string(),
            Some((a, _)) => a.to_string(),
            None => c.clone(),
        };
        (1..=base.len()).rev().find_map(|n| self.prefixes.get(&base[..n])).and_then(|&i| self.countries.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_exact_calls() {
        let db = "Romania:                  20:  28:  EU:   45.80:   -24.97:    -2.0:  YO:\n    YO,YP,YQ,YR;\nFrance:                   14:  27:  EU:   46.00:    -2.00:    -1.0:  F:\n    F,HW,HX,HY,TH,TM,TO(8)[11],TP,=TM0X;\n";
        let c = Cty::parse(db);
        assert_eq!(c.find("YO3RAD").unwrap().name, "Romania");
        let f = c.find("F4ABC").unwrap();
        assert_eq!((f.name.as_str(), f.lat, f.lon), ("France", 46.0, 2.0));
        assert_eq!(c.find("TO7AB").unwrap().name, "France");
        assert_eq!(c.find("tm0x").unwrap().name, "France");
        assert!(c.find("K1ABC").is_none());
    }
}
