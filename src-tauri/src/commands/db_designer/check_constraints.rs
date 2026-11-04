/// Rewrite SQL identifier tokens without touching quoted string literals.
pub(super) fn rewrite(sql: &str, rename: Option<(&str, &str)>, quote: Option<char>) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            out.push(ch); i += 1;
            while i < chars.len() {
                let ch = chars[i]; out.push(ch); i += 1;
                if ch == '\\' && i < chars.len() { out.push(chars[i]); i += 1; }
                else if ch == '\'' {
                    if chars.get(i) == Some(&'\'') { out.push('\''); i += 1; }
                    else { break; }
                }
            }
        } else if ch == '`' || ch == '"' {
            let original = ch;
            let mut ident = String::new(); i += 1;
            while i < chars.len() {
                let ch = chars[i]; i += 1;
                if ch == original {
                    if chars.get(i) == Some(&original) { ident.push(ch); i += 1; }
                    else { break; }
                } else { ident.push(ch); }
            }
            let ident = rename.filter(|(old, _)| *old == ident).map(|(_, new)| new).unwrap_or(&ident);
            let q = quote.unwrap_or(original);
            out.push(q); out.push_str(&ident.replace(q, &format!("{q}{q}"))); out.push(q);
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i; i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            let ident: String = chars[start..i].iter().collect();
            let function = chars[i..].iter().find(|ch| !ch.is_whitespace()) == Some(&'(');
            out.push_str(rename.filter(|(old, _)| !function && *old == ident).map(|(_, new)| new).unwrap_or(&ident));
        } else { out.push(ch); i += 1; }
    }
    out
}
