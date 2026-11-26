//! A bound on how deep a TS/JS source nests, read before Oxc parses it.
//!
//! Oxc's parser is recursive descent, and every walk over its syntax tree
//! recurses once per level. A source nested tens of thousands of levels deep
//! overflows any thread stack, and a stack overflow aborts the process rather
//! than unwinding. [`nesting_past_limit`] reads the source once, in a loop, and
//! finds where its nesting passes [`MAX_NESTING_DEPTH`], so such a file is
//! reported and never reaches the parser.
//!
//! The count is lexical and leans high. Each open bracket, brace, parenthesis
//! and template substitution counts one level. Inside one, every operator
//! character, every closed group and every keyword that can nest a node (`new`,
//! `typeof`, `if`, `else`, `keyof`, `extends`, `as` and the like) counts one
//! more, because a chain such as `a + a`, `a.b.c`, `f()()`, `a[0][0]` or
//! `a ? b : c ? d : e` builds a syntax tree as deep as the chain is long.
//!
//! A statement boundary clears its frame's count: a `;`; a word after a closed
//! `{}` group, other than `else`, `catch`, `finally` and `while`; and a word
//! after a line break that follows a value, other than a binary or statement
//! keyword, as automatic semicolon insertion reads it. A `,` clears the count
//! too, except for the type-argument or JSX angles still open in that frame,
//! since `Map<K, Map<K, V>>` nests across commas. A single `|` or `&` after a
//! string literal does not count: a union of string literal types is one flat
//! node, and generated declarations list thousands of members.

/// Deepest nesting a TS/JS source may reach before it is reported and not
/// parsed. A parse and its walks at this depth fit in
/// [`crate::jobs::WORKER_STACK_BYTES`] with room to spare.
pub(crate) const MAX_NESTING_DEPTH: u32 = 10_000;

/// Keywords that can nest one node inside another at the same bracket level.
const NESTING_KEYWORDS: &[&[u8]] = &[
    b"as",
    b"asserts",
    b"await",
    b"delete",
    b"do",
    b"else",
    b"extends",
    b"for",
    b"if",
    b"in",
    b"infer",
    b"instanceof",
    b"is",
    b"keyof",
    b"new",
    b"of",
    b"readonly",
    b"satisfies",
    b"typeof",
    b"unique",
    b"void",
    b"while",
    b"with",
    b"yield",
];

/// Words after a closed `{}` group that continue its statement.
const BRACE_CONTINUATIONS: &[&[u8]] = &[b"catch", b"else", b"finally", b"while"];

/// Words after a line break that continue the expression or statement before
/// it, so the line break is no statement boundary there.
const LINE_CONTINUATIONS: &[&[u8]] = &[
    b"as",
    b"do",
    b"else",
    b"extends",
    b"for",
    b"if",
    b"implements",
    b"in",
    b"instanceof",
    b"of",
    b"satisfies",
    b"while",
    b"with",
];

/// Keywords after which a `/` starts a regular expression, not a division.
const REGEX_PREFIX_KEYWORDS: &[&[u8]] = &[
    b"await",
    b"case",
    b"delete",
    b"do",
    b"else",
    b"extends",
    b"in",
    b"instanceof",
    b"new",
    b"of",
    b"return",
    b"throw",
    b"typeof",
    b"void",
    b"yield",
];

/// The byte offset where `source` first nests deeper than `limit`, or `None`
/// when it stays within it.
pub(crate) fn nesting_past_limit(source: &str, limit: u32) -> Option<usize> {
    Scanner::new(source.as_bytes(), limit).run()
}

struct Frame {
    /// Nesting operators and keywords since the statement or list item began.
    count: u32,
    /// `<` opened and not closed by `</` or `/>` since the statement began.
    angles: u32,
    /// The frame is a template substitution, so its `}` resumes the template.
    template: bool,
    /// The frame opened with `{`.
    brace: bool,
}

struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
    limit: u32,
    /// The root frame is the file itself and never closes.
    frames: Vec<Frame>,
    /// Sum of every frame's `count`.
    counted: u32,
    /// Whether the last token was a value, so a `/` after it divides.
    after_value: bool,
    /// The last token closed a `{}` group.
    after_brace_group: bool,
    /// A line break followed a value since the last token.
    after_line_break: bool,
    /// The last token was a string literal.
    after_string: bool,
}

impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8], limit: u32) -> Self {
        Self {
            bytes,
            at: 0,
            limit,
            frames: vec![Frame {
                count: 0,
                angles: 0,
                template: false,
                brace: false,
            }],
            counted: 0,
            after_value: false,
            after_brace_group: false,
            after_line_break: false,
            after_string: false,
        }
    }

    fn depth(&self) -> u32 {
        (self.frames.len() as u32 - 1).saturating_add(self.counted)
    }

    fn top(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the root frame never closes")
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    /// Count one nesting token in the innermost frame; `true` when that passes
    /// the limit.
    fn bump(&mut self) -> bool {
        self.top().count += 1;
        self.counted = self.counted.saturating_add(1);
        self.depth() > self.limit
    }

    fn reset_top(&mut self, keep_angles: bool) {
        let frame = self.top();
        let old = frame.count;
        let new = if keep_angles {
            frame.angles.saturating_mul(2)
        } else {
            frame.angles = 0;
            0
        };
        frame.count = new;
        self.counted = self.counted - old + new;
    }

    /// Open a frame; `true` when that passes the limit.
    fn open(&mut self, template: bool, brace: bool) -> bool {
        self.frames.push(Frame {
            count: 0,
            angles: 0,
            template,
            brace,
        });
        self.depth() > self.limit
    }

    /// Close the innermost frame. A closed group counts one in the frame
    /// around it; a closed template substitution resumes its template. A stray
    /// closer at the root is ignored. `true` when that passes the limit.
    fn close(&mut self) -> bool {
        if self.frames.len() == 1 {
            self.after_value = true;
            return false;
        }
        let frame = self.frames.pop().expect("more than the root frame");
        self.counted -= frame.count;
        if frame.template {
            return self.scan_template();
        }
        self.after_value = true;
        self.after_brace_group = frame.brace;
        self.bump()
    }

    fn run(mut self) -> Option<usize> {
        if self.bytes.starts_with(b"#!") {
            self.skip_line();
        }
        while let Some(byte) = self.peek(0) {
            let start = self.at;
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                if byte == b'\n' && self.after_value {
                    self.after_line_break = true;
                }
                self.at += 1;
                continue;
            }
            if is_identifier_byte(byte) && !byte.is_ascii_digit() {
                if self.scan_word() {
                    return Some(start);
                }
                continue;
            }
            if self.after_brace_group && !is_continuation_byte(byte) {
                self.reset_top(false);
            }
            self.after_brace_group = false;
            self.after_line_break = false;
            let after_string = std::mem::take(&mut self.after_string);
            let past = match byte {
                b'/' if self.peek(1) == Some(b'/') => {
                    self.skip_line();
                    false
                }
                b'/' if self.peek(1) == Some(b'*') => {
                    self.skip_block_comment();
                    false
                }
                b'/' if self.peek(1) == Some(b'>') => {
                    self.at += 2;
                    let frame = self.top();
                    frame.angles = frame.angles.saturating_sub(1);
                    self.after_value = true;
                    false
                }
                b'/' if !self.after_value => {
                    self.skip_regex();
                    self.after_value = true;
                    false
                }
                b'\'' | b'"' => {
                    self.skip_string(byte);
                    self.after_value = true;
                    self.after_string = true;
                    false
                }
                // A union or intersection of string literal types is one flat
                // node however many members it lists.
                b'|' | b'&'
                    if after_string
                        && !matches!(self.peek(1), Some(next) if next == byte || next == b'=') =>
                {
                    self.at += 1;
                    self.after_value = false;
                    false
                }
                b'`' => {
                    self.at += 1;
                    self.bump() || self.scan_template()
                }
                b'(' | b'[' | b'{' => {
                    self.at += 1;
                    self.after_value = false;
                    self.open(false, byte == b'{')
                }
                b')' | b']' | b'}' => {
                    self.at += 1;
                    self.close()
                }
                b';' => {
                    self.at += 1;
                    self.reset_top(false);
                    self.after_value = false;
                    false
                }
                b',' => {
                    self.at += 1;
                    self.reset_top(true);
                    self.after_value = false;
                    false
                }
                b'<' if self.peek(1) == Some(b'/') => {
                    self.at += 2;
                    let frame = self.top();
                    frame.angles = frame.angles.saturating_sub(1);
                    self.after_value = false;
                    self.bump()
                }
                b'<' => {
                    self.at += 1;
                    let frame = self.top();
                    frame.angles = frame.angles.saturating_add(1);
                    self.after_value = false;
                    self.bump()
                }
                b'.' if self.peek(1).is_some_and(|next| next.is_ascii_digit()) => {
                    self.skip_number();
                    self.after_value = true;
                    false
                }
                b'.' | b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'&' | b'|' | b'^' | b'!'
                | b'~' | b'>' | b'?' | b':' | b'@' => {
                    self.at += 1;
                    self.after_value = false;
                    self.bump()
                }
                b'0'..=b'9' => {
                    self.skip_number();
                    self.after_value = true;
                    false
                }
                _ => {
                    self.at += 1;
                    false
                }
            };
            if past {
                return Some(start);
            }
        }
        None
    }

    fn scan_word(&mut self) -> bool {
        let start = self.at;
        while self.peek(0).is_some_and(is_identifier_byte) {
            self.at += 1;
        }
        let word = &self.bytes[start..self.at];
        if (self.after_brace_group && !BRACE_CONTINUATIONS.contains(&word))
            || (self.after_line_break && !LINE_CONTINUATIONS.contains(&word))
        {
            self.reset_top(false);
        }
        self.after_brace_group = false;
        self.after_line_break = false;
        self.after_string = false;
        self.after_value = !REGEX_PREFIX_KEYWORDS.contains(&word);
        NESTING_KEYWORDS.contains(&word) && self.bump()
    }

    /// Scan template text up to its end or its next substitution; `true` when
    /// the substitution passes the limit.
    fn scan_template(&mut self) -> bool {
        while let Some(byte) = self.peek(0) {
            match byte {
                b'\\' => self.at += 2,
                b'`' => {
                    self.at += 1;
                    self.after_value = true;
                    return false;
                }
                b'$' if self.peek(1) == Some(b'{') => {
                    self.at += 2;
                    self.after_value = false;
                    return self.open(true, false);
                }
                _ => self.at += 1,
            }
        }
        false
    }

    fn skip_line(&mut self) {
        while self.peek(0).is_some_and(|byte| byte != b'\n') {
            self.at += 1;
        }
    }

    fn skip_block_comment(&mut self) {
        self.at += 2;
        while self.at < self.bytes.len() {
            if self.peek(0) == Some(b'*') && self.peek(1) == Some(b'/') {
                self.at += 2;
                return;
            }
            self.at += 1;
        }
    }

    /// A string ends at its closing quote or, unterminated, at the line end.
    fn skip_string(&mut self, quote: u8) {
        self.at += 1;
        while let Some(byte) = self.peek(0) {
            match byte {
                b'\\' => self.at += 2,
                b'\n' => return,
                _ if byte == quote => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    fn skip_regex(&mut self) {
        self.at += 1;
        let mut in_class = false;
        while let Some(byte) = self.peek(0) {
            match byte {
                b'\\' => self.at += 2,
                b'\n' => return,
                b'[' => {
                    in_class = true;
                    self.at += 1;
                }
                b']' => {
                    in_class = false;
                    self.at += 1;
                }
                b'/' if !in_class => {
                    self.at += 1;
                    while self.peek(0).is_some_and(is_identifier_byte) {
                        self.at += 1;
                    }
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    fn skip_number(&mut self) {
        while self
            .peek(0)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
        {
            self.at += 1;
        }
    }
}

/// Bytes that continue an expression after a closed `{}` group.
fn is_continuation_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'.' | b'?'
            | b'('
            | b'['
            | b'`'
            | b','
            | b';'
            | b')'
            | b']'
            | b'}'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'&'
            | b'|'
            | b'^'
            | b'!'
            | b'<'
            | b'>'
            | b':'
    )
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'#' | b'\\') || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth_reached(source: &str) -> u32 {
        (0..=MAX_NESTING_DEPTH)
            .find(|limit| nesting_past_limit(source, *limit).is_none())
            .expect("the source nests within the default limit")
    }

    #[test]
    fn brackets_and_chains_count_one_level_each() {
        assert_eq!(depth_reached("x;"), 0);
        assert_eq!(depth_reached("((a));"), 2);
        assert_eq!(depth_reached("a.b.c.d;"), 3);
        assert_eq!(depth_reached("!!!a;"), 3);
    }

    #[test]
    fn statements_and_list_items_do_not_add_up() {
        let flat = format!(
            "x = [{}];\n{}",
            "1,".repeat(50_000),
            "a + b;\n".repeat(50_000)
        );
        assert!(depth_reached(&flat) <= 3);
    }

    #[test]
    fn strings_comments_and_regular_expressions_do_not_count() {
        let source = r#"
            const a = "((((((" + '[[[[' ;
            // {{{{{{{{
            /* (((((((( */
            const r = /[(((]\/(((/g.test(a);
            const t = `((( ${a} [[[`;
        "#;
        assert!(depth_reached(source) <= 4, "{}", depth_reached(source));
    }

    #[test]
    fn a_union_of_string_literals_is_flat() {
        let members = (0..20_000).map(|i| format!("'m{i}'")).collect::<Vec<_>>();
        let source = format!("type Name =\n  | {};", members.join("\n  | "));
        assert!(depth_reached(&source) <= 2, "{}", depth_reached(&source));
        let chain = format!("x = {}a;", "'s' || ".repeat(2_000));
        assert!(depth_reached(&chain) >= 2_000);
    }

    #[test]
    fn type_arguments_nest_across_commas() {
        let levels = 2_000;
        let source = format!(
            "type T = {}V{};",
            "Map<K, ".repeat(levels),
            ">".repeat(levels)
        );
        assert!(depth_reached(&source) >= levels as u32);
    }

    #[test]
    fn template_substitutions_nest() {
        let levels = 2_000;
        let source = format!("x = {}a{};", "`${".repeat(levels), "}`".repeat(levels));
        assert!(depth_reached(&source) >= levels as u32);
    }

    #[test]
    fn the_offset_is_where_the_limit_is_passed() {
        let source = "x = ((((a))));";
        assert_eq!(nesting_past_limit(source, 2), Some(5));
        assert_eq!(nesting_past_limit(source, 5), None);
    }
}
