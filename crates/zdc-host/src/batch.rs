//! Reading `[[endpoint, args], ...]` — the body one handler's transaction
//! is posted as.
//!
//! # Why this scans rather than deserialises
//!
//! Everywhere else in this crate a request body is handed to the engine's
//! own `JSON.parse` and never touched by Rust (see `js_string` in
//! [`crate::bindings`]: the body is spliced in as an inert literal, never
//! as source). That cannot work here, because the *names* in the batch
//! have to be resolved to endpoints before anything runs — a stale tab
//! posting one renamed endpoint alongside three live ones must get none of
//! its handler, not three quarters of it.
//!
//! So Rust has to see the names. It does not have to see anything else:
//! each argument list is carried through as **the source text it arrived
//! as** and handed to `JSON.parse` in the engine exactly as a single
//! call's body already is. This scanner therefore finds two things — where
//! a string ends and where a bracketed value ends — and understands
//! nothing about numbers, records or `{"$map":[[k,v]]}`. The wire format
//! keeps exactly one definition, in `wire.js`, which is the property that
//! stopped a durable `Map` arriving as `{}`.

/// One entry: the endpoint the browser named, and its arguments as the
/// JSON text they arrived as.
pub type Call = (String, String);

/// Split a batch body into its calls.
///
/// `Err` carries the sentence a developer reads. The body is
/// attacker-controlled, so every refusal here is a 400 rather than a
/// panic, and "the body is not the shape this endpoint takes" is the whole
/// of what it can say without echoing the body back.
pub fn parse(body: &str) -> Result<Vec<Call>, String> {
    let mut scan = Scan {
        bytes: body.as_bytes(),
        at: 0,
    };
    scan.space();
    scan.take(b'[')
        .ok_or("a transaction body must be an array of writes")?;
    let mut calls = Vec::new();
    scan.space();
    if scan.take(b']').is_some() {
        scan.space();
        return match scan.done() {
            true => Ok(calls),
            false => Err("a transaction body has trailing text after the array".to_string()),
        };
    }
    loop {
        scan.space();
        scan.take(b'[')
            .ok_or("every write in a transaction is a `[name, arguments]` pair")?;
        scan.space();
        let name = scan
            .string()
            .ok_or("a write's first element must be the endpoint name, as a string")?;
        scan.space();
        scan.take(b',')
            .ok_or("a write names an endpoint and then its arguments")?;
        scan.space();
        let arguments = scan
            .value()
            .filter(|text| text.starts_with('['))
            .ok_or("a write's arguments must be an array")?
            .to_string();
        scan.space();
        scan.take(b']')
            .ok_or("a write is a pair, and this one carried more than two elements")?;
        calls.push((name, arguments));
        scan.space();
        if scan.take(b',').is_some() {
            continue;
        }
        scan.take(b']')
            .ok_or("the transaction's array of writes is not closed")?;
        break;
    }
    scan.space();
    if !scan.done() {
        return Err("a transaction body has trailing text after the array".to_string());
    }
    Ok(calls)
}

struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn take(&mut self, byte: u8) -> Option<()> {
        if self.peek() == Some(byte) {
            self.at += 1;
            return Some(());
        }
        None
    }

    /// A JSON string, decoded.
    ///
    /// Escapes are decoded because a name is compared against the endpoint
    /// table by equality, and `"visits.incr"` is the same name as
    /// `"visits.incr"` to every JSON reader in the world. Getting that
    /// wrong would be a 404 rather than a hole, but a 404 for a request
    /// that names a real endpoint is still a bug.
    fn string(&mut self) -> Option<String> {
        self.take(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            self.at += 1;
            match byte {
                b'"' => return Some(out),
                b'\\' => {
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.bytes.get(self.at..self.at + 4)?;
                            self.at += 4;
                            let code =
                                u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            // A lone surrogate is not a scalar value and
                            // cannot be a character. No endpoint name
                            // contains one; the replacement simply means
                            // the lookup will not match.
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
                _ => {
                    // Multi-byte UTF-8 arrives one byte at a time here, so
                    // the whole sequence is copied rather than decoded: the
                    // input was `&str`, so it is valid UTF-8 already and
                    // reassembling the bytes cannot produce anything else.
                    let start = self.at - 1;
                    while self.peek().is_some_and(|next| (0x80..0xc0).contains(&next)) {
                        self.at += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).ok()?);
                }
            }
        }
    }

    /// One JSON value, as the text it occupies.
    ///
    /// Nesting is tracked through brackets and braces, and a bracket
    /// inside a string does not count — which is the one case a naive
    /// bracket counter gets wrong, and the case a hostile body would use.
    fn value(&mut self) -> Option<&'a str> {
        let start = self.at;
        let mut depth = 0usize;
        loop {
            let byte = self.peek()?;
            match byte {
                b'"' => {
                    self.string()?;
                    continue;
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.at += 1;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                b',' if depth == 0 => break,
                _ => {}
            }
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.at]).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_splits_into_its_calls_with_the_arguments_untouched() {
        assert_eq!(
            parse("[[\"votes.incr\",[1]],[\"winner.set\",[\"ada\"]]]"),
            Ok(vec![
                ("votes.incr".to_string(), "[1]".to_string()),
                ("winner.set".to_string(), "[\"ada\"]".to_string()),
            ])
        );
    }

    #[test]
    fn an_argument_that_is_a_map_survives_as_the_text_it_arrived_as() {
        // The `{"$map":[[k,v]]}` form is load-bearing and this scanner
        // must not have an opinion about it. Handing the text through is
        // what keeps one definition of the wire format.
        assert_eq!(
            parse("[[\"held.set\",[{\"$map\":[[\"a\",1],[\"b\",2]]}]]]"),
            Ok(vec![(
                "held.set".to_string(),
                "[{\"$map\":[[\"a\",1],[\"b\",2]]}]".to_string()
            )])
        );
    }

    #[test]
    fn a_bracket_inside_a_string_argument_does_not_end_the_arguments() {
        // The case a bracket counter gets wrong, and therefore the case a
        // hostile body would use to make one call's arguments swallow the
        // next call's name.
        assert_eq!(
            parse("[[\"held.set\",[\"]],[\\\"other.set\\\",[1\"]]]"),
            Ok(vec![(
                "held.set".to_string(),
                "[\"]],[\\\"other.set\\\",[1\"]".to_string()
            )])
        );
    }

    #[test]
    fn whitespace_between_every_token_is_accepted() {
        assert_eq!(
            parse(" [ [ \"a.incr\" , [ 1 ] ] , [ \"b.incr\" , [ 2 ] ] ] "),
            Ok(vec![
                ("a.incr".to_string(), "[ 1 ]".to_string()),
                ("b.incr".to_string(), "[ 2 ]".to_string()),
            ])
        );
    }

    #[test]
    fn an_empty_batch_is_an_empty_list_and_not_an_error() {
        // A handler whose only write sits inside an `if` that did not fire
        // has nothing to commit, and refusing it would turn a correct
        // program into a reported failure.
        assert_eq!(parse("[]"), Ok(Vec::new()));
        assert_eq!(parse("  [ ]  "), Ok(Vec::new()));
    }

    #[test]
    fn an_escaped_name_is_the_name_it_escapes() {
        assert_eq!(
            parse("[[\"visits\\u002Eincr\",[1]]]"),
            Ok(vec![("visits.incr".to_string(), "[1]".to_string())])
        );
    }

    #[test]
    fn a_name_outside_ascii_survives_intact() {
        assert_eq!(
            parse("[[\"café.set\",[1]]]"),
            Ok(vec![("café.set".to_string(), "[1]".to_string())])
        );
    }

    #[test]
    fn every_malformed_shape_is_refused_rather_than_guessed() {
        // Each of these could be "repaired" into something plausible, and
        // each repair would run a write the browser did not ask for.
        for body in [
            "",
            "null",
            "{}",
            "[\"votes.incr\"]",
            "[[\"votes.incr\"]]",
            "[[\"votes.incr\",1]]",
            "[[1,[1]]]",
            "[[\"votes.incr\",[1]]",
            "[[\"votes.incr\",[1],[2]]]",
            "[[\"votes.incr\",[1]]] extra",
        ] {
            assert!(parse(body).is_err(), "`{body}` was accepted");
        }
    }
}
