//! Bounded stream conversion to the file formats read by the pinned daemon.
//! The daemon remains responsible for matching and regular-expression compilation.
use super::model::Format;
use std::io::Write;

pub(super) struct Normalizer {
    format: Format,
    line: Vec<u8>,
    lines: usize,
    entries: usize,
    pub bytes: u64,
    max: u64,
}

impl Normalizer {
    pub fn new(format: Format, max: u64) -> Self {
        Self {
            format,
            line: Vec::new(),
            lines: 0,
            entries: 0,
            bytes: 0,
            max,
        }
    }
    pub fn push(&mut self, chunk: &[u8], output: &mut impl Write) -> Result<(), String> {
        for byte in chunk {
            if *byte == b'\n' {
                self.line(output)?;
            } else {
                if self.line.len() >= 8192 {
                    return Err("List contains a line above 8 KiB".into());
                }
                self.line.push(*byte);
            }
        }
        Ok(())
    }
    fn line(&mut self, output: &mut impl Write) -> Result<(), String> {
        self.lines += 1;
        if self.lines > 2_000_000 {
            return Err("List exceeds two million lines".into());
        }
        let buffer = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&buffer)
            .map_err(|_| "List must be UTF-8 text")?
            .trim_start_matches('\u{feff}')
            .trim_end_matches('\r');
        let line = if self.format == Format::Regexp {
            line
        } else {
            line.trim()
        };
        if line.contains('\0')
            || (self.format != Format::Regexp && (line.contains('<') || line.contains('>')))
            || line.to_ascii_lowercase().starts_with("<html")
            || line.to_ascii_lowercase().starts_with("<!doctype")
        {
            return Err("Response is not a supported text list (HTML/NUL detected)".into());
        }
        if !line.is_empty() && !line.starts_with('#') {
            let value = if self.format == Format::Regexp {
                line
            } else {
                line.split('#').next().unwrap_or("").trim()
            };
            match self.format {
                Format::Hosts => {
                    let mut fields = value.split_whitespace();
                    let ip = fields.next().ok_or("Missing hosts address")?;
                    ip.parse::<std::net::IpAddr>()
                        .map_err(|_| "Invalid hosts address")?;
                    let names = fields.collect::<Vec<_>>();
                    if names.is_empty() {
                        return Err("Hosts entry has no name".into());
                    }
                    // These are the addresses accepted by OpenSnitch's hosts reader.
                    if matches!(ip, "0.0.0.0" | "127.0.0.1") {
                        for name in names {
                            self.domain(name, output)?;
                        }
                    }
                }
                Format::Domains => self.domain(value, output)?,
                Format::Ips | Format::Nets => {
                    let valid = value.parse::<std::net::IpAddr>().is_ok()
                        || value.split_once('/').is_some_and(|(ip, prefix)| {
                            ip.parse::<std::net::IpAddr>().ok().is_some_and(|ip| {
                                prefix
                                    .parse::<u8>()
                                    .is_ok_and(|p| p <= if ip.is_ipv4() { 32 } else { 128 })
                            })
                        });
                    if !valid {
                        return Err(format!("Line {} is not valid IP/CIDR data", self.lines));
                    }
                    self.write(value, output)?;
                }
                Format::Md5 => {
                    let hash = value.split_whitespace().next().unwrap_or("");
                    if hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err("Invalid MD5 entry".into());
                    }
                    self.write(&hash.to_ascii_lowercase(), output)?;
                }
                Format::Regexp => self.write(value, output)?,
            }
        }
        self.line = buffer;
        self.line.clear();
        Ok(())
    }
    fn domain(&mut self, name: &str, output: &mut impl Write) -> Result<(), String> {
        if matches!(
            name,
            "local" | "localhost" | "localhost.localdomain" | "broadcasthost"
        ) {
            return Ok(());
        }
        if name.is_empty()
            || name.len() > 253
            || !name.is_ascii()
            || name.chars().any(char::is_whitespace)
            || name.contains(['/', ':', '{', '}', '\\'])
        {
            return Err("Unsupported domain/pattern entry; use ASCII/IDNA names and separate brace alternatives".into());
        }
        let pattern = name
            .strip_prefix("*.")
            .or_else(|| name.strip_prefix('.'))
            .unwrap_or(name);
        if pattern.split('.').any(str::is_empty) {
            return Err("Domain pattern contains an empty label".into());
        }
        let mut brackets = 0;
        for c in pattern.chars() {
            match c {
                '[' => {
                    if brackets != 0 {
                        return Err("Nested domain glob brackets".into());
                    }
                    brackets = 1;
                }
                ']' => {
                    if brackets != 1 {
                        return Err("Unmatched domain glob bracket".into());
                    }
                    brackets = 0;
                }
                _ => {}
            }
        }
        if brackets != 0 {
            return Err("Unclosed domain glob bracket".into());
        }
        self.write(&format!("0.0.0.0 {}", name.to_ascii_lowercase()), output)
    }
    fn write(&mut self, value: &str, output: &mut impl Write) -> Result<(), String> {
        let bytes = self.bytes.saturating_add(value.len() as u64 + 1);
        if bytes > self.max {
            return Err("Normalized list exceeds the configured byte limit".into());
        }
        output
            .write_all(value.as_bytes())
            .and_then(|_| output.write_all(b"\n"))
            .map_err(|e| e.to_string())?;
        self.bytes = bytes;
        self.entries += 1;
        Ok(())
    }
    pub fn finish(&mut self, output: &mut impl Write) -> Result<(), String> {
        if !self.line.is_empty() {
            self.line(output)?;
        }
        if self.entries == 0 {
            return Err("No usable list entries found; previous data retained".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_domains_have_canonical_hosts_rows() {
        let mut normalizer = Normalizer::new(Format::Domains, 1024);
        let mut data = Vec::new();
        for part in [b"Exam".as_slice(), b"ple.com\r", b"\n*.wild.example"] {
            normalizer.push(part, &mut data).unwrap();
        }
        normalizer.finish(&mut data).unwrap();
        assert_eq!(data, b"0.0.0.0 example.com\n0.0.0.0 *.wild.example\n");
    }
    #[test]
    fn normalized_output_growth_and_lines_have_independent_caps() {
        let mut normalizer = Normalizer::new(Format::Domains, 16);
        let mut data = Vec::new();
        assert!(
            normalizer
                .push(b"example.com\n", &mut data)
                .unwrap_err()
                .contains("Normalized")
        );
        let mut normalizer = Normalizer::new(Format::Domains, 100000);
        assert!(
            normalizer
                .push(&vec![b'a'; 8193], &mut data)
                .unwrap_err()
                .contains("8 KiB")
        );
    }
    #[test]
    fn regexp_literals_and_whitespace_survive_normalization() {
        let mut normalizer = Normalizer::new(Format::Regexp, 1024);
        let mut data = Vec::new();
        normalizer
            .push(b"  ^(?P<name>a#b)$  \r\n", &mut data)
            .unwrap();
        normalizer.finish(&mut data).unwrap();
        assert_eq!(data, b"  ^(?P<name>a#b)$  \n");
    }
    #[test]
    fn unusable_hosts_and_bad_patterns_do_not_publish_empty_data() {
        let mut normalizer = Normalizer::new(Format::Hosts, 1024);
        let mut data = Vec::new();
        normalizer
            .push(b"127.0.0.1 localhost\n::1 localhost\n", &mut data)
            .unwrap();
        assert!(
            normalizer
                .finish(&mut data)
                .unwrap_err()
                .contains("No usable")
        );
        assert!(data.is_empty());
        let mut normalizer = Normalizer::new(Format::Domains, 1024);
        assert!(normalizer.push(b"{www,api}.example\n", &mut data).is_err());
    }
}
