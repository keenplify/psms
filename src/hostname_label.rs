pub fn suggested_name(name: &str) -> String {
    let mut s = String::new();
    for b in name.split('.').next().unwrap_or("").bytes() {
        if s.len() >= 48 {
            break;
        }
        if b.is_ascii_alphanumeric() {
            s.push(b.to_ascii_lowercase() as char)
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-')
        }
    }
    let s = s.trim_end_matches('-');
    if s.is_empty() {
        format!("psms-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
    } else {
        s.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_names_are_dns_labels() {
        assert_eq!(suggested_name("  My__Laptop.local"), "my-laptop");
        assert!(suggested_name("🐈").starts_with("psms-"));
        assert!(suggested_name(&"a".repeat(100)).len() <= 63);
    }
}
