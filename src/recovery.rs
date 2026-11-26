use bip39::{Language, Mnemonic};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};

pub fn key_from_backup(value: &str) -> Result<String, &'static str> {
    let value = value.trim_end_matches(['\r', '\n']);
    if value.len() == 32 && value.is_ascii() {
        return Ok(value.to_owned());
    }
    let phrase = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let mnemonic = Mnemonic::parse_in(Language::English, phrase)
        .map_err(|_| "Enter the 12 recovery words in order or the original 32-character key.")?;
    let entropy = mnemonic.to_entropy();
    if entropy.len() != 16 {
        return Err("This archive uses a 12-word recovery phrase.");
    }
    Ok(entropy.iter().map(|b| format!("{b:02x}")).collect())
}

fn text(c: &mut Content, x: f32, y: f32, size: f32, value: &str) {
    c.begin_text()
        .set_font(Name(b"Body"), size)
        .next_line(x, y)
        .show(Str(value.as_bytes()))
        .end_text();
}

pub fn printable(entropy: &[u8; 16], authority: &str, username: Option<&str>) -> Vec<u8> {
    let mut pdf = Pdf::new();
    pdf.catalog(Ref::new(1)).pages(Ref::new(2));
    pdf.pages(Ref::new(2)).kids([Ref::new(3)]).count(1);
    let mut page = pdf.page(Ref::new(3));
    page.parent(Ref::new(2))
        .media_box(Rect::new(0., 0., 595., 842.))
        .contents(Ref::new(5));
    page.resources().fonts().pair(Name(b"Body"), Ref::new(4));
    page.finish();
    pdf.type1_font(Ref::new(4))
        .base_font(Name(b"Helvetica-Bold"));
    let mut c = Content::new();
    c.set_fill_rgb(0.06, 0.12, 0.13)
        .rect(0., 734., 595., 108.)
        .fill_nonzero();
    c.set_fill_rgb(0.76, 0.95, 0.3);
    text(&mut c, 44., 783., 28., "pSMS");
    text(&mut c, 382., 783., 13., "ARCHIVE RECOVERY KIT");
    c.set_fill_rgb(1., 1., 1.);
    text(&mut c, 45., 760., 11., "Papamo Software");
    c.set_fill_rgb(0.06, 0.12, 0.13);
    text(&mut c, 45., 680., 30., "Your archive backup");
    if let Some(user) = username {
        text(&mut c, 45., 647., 13., &format!("Papamo account: {user}"));
    } else {
        text(&mut c, 45., 647., 13., "Self-hosted archive");
    }
    for (i, line) in authority.as_bytes().chunks(68).enumerate() {
        text(
            &mut c,
            45.,
            622. - i as f32 * 13.,
            11.,
            std::str::from_utf8(line).unwrap_or(""),
        );
    }
    c.set_fill_rgb(0.94, 0.96, 0.91)
        .rect(44., 274., 507., 264.)
        .fill_nonzero();
    c.set_fill_rgb(0.06, 0.12, 0.13);
    text(&mut c, 64., 507., 13., "RECOVERY PHRASE");
    for (i, word) in Mnemonic::from_entropy(entropy)
        .expect("fixed-size entropy")
        .words()
        .enumerate()
    {
        text(
            &mut c,
            64. + (i % 3) as f32 * 162.,
            461. - (i / 3) as f32 * 46.,
            15.,
            &format!("{:02}    {word}", i + 1),
        );
    }
    text(&mut c, 45., 223., 19., "Restore your archive");
    text(
        &mut c,
        45.,
        196.,
        12.,
        "Open your pSMS archive and enter the 12 words in the order shown.",
    );
    text(
        &mut c,
        45.,
        171.,
        12.,
        "Keep this sheet private. Anyone with it and your ciphertext can read your SMS.",
    );
    text(
        &mut c,
        45.,
        101.,
        13.,
        "Keep a separate backup of your database.",
    );
    text(
        &mut c,
        45.,
        78.,
        11.,
        "These words recover the encryption key. Your server holds the messages.",
    );
    pdf.stream(Ref::new(5), &c.finish());
    pdf.finish()
}
#[cfg(test)]
mod tests {
    #[test]
    fn restores_existing_ascii_key_from_standard_phrase() {
        let phrase = format!("{}about", "abandon ".repeat(11));
        assert_eq!(super::key_from_backup(&phrase).unwrap(), "00".repeat(16));
        assert!(super::key_from_backup(&"abandon ".repeat(12)).is_err());
    }
}
