pub(crate) struct XmlWriter<'a> {
    state: State,
    /// Number of start tags calls without matching end tag.
    started: usize,
    /// Output string to append the resulting XML to.
    w: &'a mut String,
}

enum State {
    Tag,
    Document,
}

impl<'a> XmlWriter<'a> {
    pub(crate) fn new(string: &'a mut String) -> XmlWriter<'a> {
        XmlWriter {
            state: State::Document,
            started: 0,
            w: string,
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.started {
            self.w.push_str("  ");
        }
    }

    pub(crate) fn start_tag(&mut self, tag: &str) {
        if let State::Tag = self.state {
            self.w.push_str(">\n");
        }

        self.indent();
        self.w.push_str("<");
        self.w.push_str(tag);
        self.started += 1;
        self.state = State::Tag;
    }

    pub(crate) fn tag_with_text(&mut self, tag: &str, text: &str) {
        if let State::Tag = self.state {
            self.w.push_str(">\n");
        }
        self.indent();
        self.w.push_str("<");
        self.w.push_str(tag);
        self.w.push_str(">");
        self.escaped_text(text);
        self.w.push_str("</");
        self.w.push_str(tag);
        self.w.push_str(">\n");

        self.state = State::Document;
    }

    fn escaped_text(&mut self, text: &str) {
        let mut i = 0;
        for (j, byte) in text.as_bytes().iter().enumerate() {
            let escaped: Option<&str> = match byte {
                b'&' => Some("&amp;"),
                b'<' => Some("&lt;"),
                b'>' => Some("&gt;"),
                _ => None,
            };
            if let Some(escaped) = escaped {
                if i != j {
                    self.w.push_str(&text[i..j]);
                }
                self.w.push_str(escaped);
                i = j + 1;
            }
        }
        if i != text.len() {
            self.w.push_str(&text[i..]);
        }
    }

    pub(crate) fn attr(&mut self, name: &str, value: &str) {
        self.w.push_str(" ");
        self.w.push_str(name);
        self.w.push_str("=\"");
        self.escaped_attr(value);
        self.w.push_str("\"");
    }

    fn escaped_attr(&mut self, value: &str) {
        let mut i = 0;
        for (j, byte) in value.as_bytes().iter().enumerate() {
            let escaped: Option<&str> = match byte {
                b'&' => Some("&amp;"),
                b'<' => Some("&lt;"),
                b'>' => Some("&gt;"),
                b'\'' => Some("&apos;"),
                b'"' => Some("&quot;"),
                _ => None,
            };
            if let Some(escaped) = escaped {
                if i != j {
                    self.w.push_str(&value[i..j]);
                }
                self.w.push_str(escaped);
                i = j + 1;
            }
        }
        if i != value.len() {
            self.w.push_str(&value[i..]);
        }
    }

    pub(crate) fn end_tag(&mut self, tag: &str) {
        self.started -= 1;
        match self.state {
            State::Tag => {
                self.w.push_str("/>\n");
            }
            State::Document => {
                self.indent();
                self.w.push_str("</");
                self.w.push_str(tag);
                self.w.push_str(">\n");
            }
        }
        self.state = State::Document;
    }
}
