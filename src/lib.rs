use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// C-ABI surface consumed by H# via `extern static [rust, "hacker_parser"]`
/// (see `bytes-io/lib.h#`). Kept in its own module so the pure-Rust API
/// above stays exactly as it was before this binding was added.
pub mod ffi;

/// Błędy parsowania
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Brak otwierającego '[' lub zamykającego ']'
    MissingBrackets,
    /// Pusty plik (brak zawartości między nawiasami)
    EmptyContent,
    /// Błąd parsowania wersji 2 (np. brak nagłówka lub sekcji)
    V2ParseError(String),
    /// Błąd parsowania wersji 3 (brak wymaganych pól)
    V3ParseError(String),
    /// Nieznany format (zawsze można spróbować v1, ale dla kompletności)
    UnknownFormat,
    /// Błąd wejścia/wyjścia przy czytaniu pliku
    IoError(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::MissingBrackets => write!(f, "Plik musi zaczynać się od '[' i kończyć ']'"),
            ParseError::EmptyContent => write!(f, "Zawartość między nawiasami jest pusta"),
            ParseError::V2ParseError(msg) => write!(f, "Błąd parsowania v2: {}", msg),
            ParseError::V3ParseError(msg) => write!(f, "Błąd parsowania v3: {}", msg),
            ParseError::UnknownFormat => write!(f, "Nieznany format pliku .hacker"),
            ParseError::IoError(msg) => write!(f, "Błąd I/O: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

/// Reprezentacja pliku .hacker (dowolna wersja)
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum HackerFile {
    V1(HackerV1),
    V2(HackerV2),
    V3(HackerV3),
}

/// Wersja 1 – dowolny ciąg znaków wewnątrz nawiasów
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HackerV1 {
    pub content: String,
}

/// Wersja 2 – nagłówek oraz sekcje z listami wartości
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HackerV2 {
    pub header: String,
    pub sections: HashMap<String, Vec<String>>,
}

/// Wersja 3 – cztery konkretne pola: nagłówek, typ, opis, ścieżka pliku
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HackerV3 {
    pub header: String,
    pub ty: String,   // "type" jest słowem kluczowym, używamy "ty"
    pub description: String,
    pub file: String,
}

/// Parsuje napis jako plik .hacker, automatycznie wykrywając wersję.
pub fn parse(input: &str) -> Result<HackerFile, ParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(ParseError::MissingBrackets);
    }
    let content = trimmed[1..trimmed.len() - 1].trim();
    if content.is_empty() {
        return Err(ParseError::EmptyContent);
    }

    // Kolejność prób: v2, v3, v1 (v1 zawsze akceptuje)
    if let Ok(v2) = parse_v2(content) {
        return Ok(HackerFile::V2(v2));
    }
    if let Ok(v3) = parse_v3(content) {
        return Ok(HackerFile::V3(v3));
    }
    // v1 jako fallback
    Ok(HackerFile::V1(HackerV1 {
        content: content.to_string(),
    }))
}

/// Parsuje zawartość (bez nawiasów) jako wersję 2.
fn parse_v2(content: &str) -> Result<HackerV2, ParseError> {
    let lines: Vec<&str> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return Err(ParseError::V2ParseError("Brak linii".to_string()));
    }

    let mut header_lines = Vec::new();
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current_section: Option<String> = None;

    let mut i = 0;
    // Najpierw zbieramy nagłówek – wszystkie linie aż do pierwszej kończącej się ':'
    while i < lines.len() {
        let line = lines[i];
        if line.ends_with(':') {
            break;
        }
        // Linia nagłówka nie może zaczynać się od "= " (wartość sekcji)
        if line.starts_with("= ") {
            return Err(ParseError::V2ParseError(
                "Wartość sekcji przed pierwszym nagłówkiem sekcji".to_string(),
            ));
        }
        header_lines.push(line);
        i += 1;
    }

    let header = header_lines.join("\n");

    // Parsowanie sekcji
    while i < lines.len() {
        let line = lines[i];
        if line.ends_with(':') {
            // nowa sekcja
            let section_name = line[..line.len() - 1].trim().to_string();
            if section_name.is_empty() {
                return Err(ParseError::V2ParseError("Pusta nazwa sekcji".to_string()));
            }
            current_section = Some(section_name.clone());
            i += 1;
        } else if line.starts_with("= ") {
            // wartość dla bieżącej sekcji
            let value = line[2..].trim().to_string();
            if let Some(ref sec) = current_section {
                sections
                    .entry(sec.clone())
                    .or_insert_with(Vec::new)
                    .push(value);
            } else {
                return Err(ParseError::V2ParseError(
                    "Wartość poza sekcją".to_string(),
                ));
            }
            i += 1;
        } else {
            // Linia, która nie jest ani nagłówkiem sekcji, ani wartością – błąd
            return Err(ParseError::V2ParseError(format!(
                "Nieoczekiwana linia: {}",
                line
            )));
        }
    }

    if sections.is_empty() {
        return Err(ParseError::V2ParseError(
            "Brak sekcji w pliku v2".to_string(),
        ));
    }

    Ok(HackerV2 { header, sections })
}

/// Parsuje zawartość (bez nawiasów) jako wersję 3.
fn parse_v3(content: &str) -> Result<HackerV3, ParseError> {
    let lines: Vec<&str> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let mut header = None;
    let mut ty = None;
    let mut description = None;
    let mut file = None;

    for line in lines {
        if line.starts_with("= ") {
            if header.is_some() {
                return Err(ParseError::V3ParseError(
                    "Powtórzony nagłówek (=)".to_string(),
                ));
            }
            header = Some(line[2..].trim().to_string());
        } else if line.starts_with("=> ") {
            if ty.is_some() {
                return Err(ParseError::V3ParseError("Powtórzony typ (=>)".to_string()));
            }
            ty = Some(line[3..].trim().to_string());
        } else if line.starts_with("-> ") {
            if description.is_some() {
                return Err(ParseError::V3ParseError(
                    "Powtórzony opis (->)".to_string(),
                ));
            }
            description = Some(line[3..].trim().to_string());
        } else if line.starts_with("--> ") {
            if file.is_some() {
                return Err(ParseError::V3ParseError(
                    "Powtórzona ścieżka pliku (-->)".to_string(),
                ));
            }
            file = Some(line[4..].trim().to_string());
        } else {
            return Err(ParseError::V3ParseError(format!(
                "Nieoczekiwana linia: {}",
                line
            )));
        }
    }

    let header = header.ok_or_else(|| ParseError::V3ParseError("Brak nagłówka (= )".to_string()))?;
    let ty = ty.ok_or_else(|| ParseError::V3ParseError("Brak typu (=> )".to_string()))?;
    let description = description
        .ok_or_else(|| ParseError::V3ParseError("Brak opisu (-> )".to_string()))?;
    let file = file.ok_or_else(|| ParseError::V3ParseError("Brak ścieżki pliku (--> )".to_string()))?;

    Ok(HackerV3 {
        header,
        ty,
        description,
        file,
    })
}

/// Parsuje plik o podanej ścieżce.
pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<HackerFile, ParseError> {
    let content = fs::read_to_string(path).map_err(|e| ParseError::IoError(e.to_string()))?;
    parse(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v1_simple() {
        let input = "[ 0.1 ]";
        let parsed = parse(input).unwrap();
        match parsed {
            HackerFile::V1(v) => assert_eq!(v.content, "0.1"),
            _ => panic!("Powinno być v1"),
        }
    }

    #[test]
    fn test_v1_multiline() {
        let input = "[\n  hello\n  world\n]";
        let parsed = parse(input).unwrap();
        match parsed {
            HackerFile::V1(v) => assert_eq!(v.content, "hello\n  world"),
            _ => panic!("Powinno być v1"),
        }
    }

    #[test]
    fn test_v2_full() {
        let input = r#"[ 
 Przyklad
 jadro:
 = xanmod
 = liquorix
 narzedzia:
 = hpm
 = hl
 wersje:
 = 0.1
 = 0.2
 = 1.1
]"#;
        let parsed = parse(input).unwrap();
        match parsed {
            HackerFile::V2(v2) => {
                assert_eq!(v2.header, "Przyklad");
                assert_eq!(v2.sections.get("jadro").unwrap(), &vec!["xanmod", "liquorix"]);
                assert_eq!(v2.sections.get("narzedzia").unwrap(), &vec!["hpm", "hl"]);
                assert_eq!(v2.sections.get("wersje").unwrap(), &vec!["0.1", "0.2", "1.1"]);
            }
            _ => panic!("Powinno być v2"),
        }
    }

    #[test]
    fn test_v3_full() {
        let input = r#"[ 
 = application
 => gui
 -> aplication for cybersecurity
 --> main.hacker
]"#;
        let parsed = parse(input).unwrap();
        match parsed {
            HackerFile::V3(v3) => {
                assert_eq!(v3.header, "application");
                assert_eq!(v3.ty, "gui");
                assert_eq!(v3.description, "aplication for cybersecurity");
                assert_eq!(v3.file, "main.hacker");
            }
            _ => panic!("Powinno być v3"),
        }
    }

    #[test]
    fn test_missing_brackets() {
        let input = "hello";
        assert!(matches!(parse(input), Err(ParseError::MissingBrackets)));
    }

    #[test]
    fn test_empty_content() {
        let input = "[]";
        assert!(matches!(parse(input), Err(ParseError::EmptyContent)));
    }

    #[test]
    fn test_v2_no_sections() {
        let input = "[ Header ]";
        // Brak sekcji – powinien spaść do v1
        let parsed = parse(input).unwrap();
        assert!(matches!(parsed, HackerFile::V1(_)));
    }
}
