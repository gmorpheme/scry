//! Machinery for parsing RTF files
//!
//! This is heavily based on / stolen from https://github.com/compenguy/rtf2text
use crate::error::Result;
use lazy_static::lazy_static;
use rtf_grimoire::tokenizer::{parse, Token};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::rc::Rc;

/// An iterator over paragraphs in an RTF file
pub type ParagraphIterator = Snipperator<std::vec::IntoIter<Token>>;

/// Parse an RTF file and return iterator over lines of text
pub fn parse_rtf_file(path: &Path) -> Result<ParagraphIterator> {
    let data = fs::read(path)?;
    parse_rtf(&data)
}

/// Parse a buffer containing rtf bytes and return and iterator over
/// lines of text
pub fn parse_rtf(data: &[u8]) -> Result<ParagraphIterator> {
    let tokens = parse(data)?;
    Ok(Snipperator::new(tokens.into_iter()))
}

/// A Snipperator is a filter that converts tokens into text snippets
pub struct Snipperator<T>
where
    T: Iterator<Item = Token>,
{
    tokens: T,
    engine: SnippetEngine,
    rtf_queue: Rc<RefCell<RtfQueueDestinationArray>>,
}

impl<T: Iterator<Item = Token>> Snipperator<T> {
    pub fn new(tokens: T) -> Self {
        let rtf_queue = Rc::new(RefCell::new(RtfQueueDestinationArray::new(
            BasicDestinationArray::default(),
        )));
        let engine = SnippetEngine::new(rtf_queue.clone());

        Snipperator {
            tokens,
            engine,
            rtf_queue,
        }
    }
}

impl<T: Iterator<Item = Token>> Iterator for Snipperator<T> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(line) = self.rtf_queue.borrow_mut().pop() {
            return Some(line);
        }

        if let Some(tok) = self.tokens.next() {
            self.engine.feed(&tok);
            return self.next();
        }

        self.rtf_queue.borrow_mut().flush()
    }
}

#[derive(Clone, Debug)]
pub enum Destination {
    Text(String),
    Bytes(Vec<u8>),
}

impl Destination {
    pub fn append_text(&mut self, new_text: &str) {
        if let Destination::Text(string) = self {
            string.push_str(new_text);
        } else {
            panic!("Programmer error: attempting to add text to a byte destination");
        }
    }

    pub fn append_bytes(&mut self, new_bytes: &[u8]) {
        if let Destination::Bytes(bytes) = self {
            bytes.extend(new_bytes);
        } else {
            panic!("Programmer error: attempting to add bytes to a text destination");
        }
    }
}

/// Destination protocol
pub trait DestinationArray {
    fn destinations(&self) -> Vec<String>;
    /// Ceate a new text destination
    fn create_text(&mut self, name: &str);
    /// Ceate a new bytes destination
    fn create_bytes(&mut self, name: &str);
    /// Write bytes to destination
    fn write(&mut self, name: &str, bytes: &[u8], encoding: Option<&'static encoding_rs::Encoding>);
    /// Read text from destination if available
    fn read_text(
        &self,
        name: &str,
        encoding: Option<&'static encoding_rs::Encoding>,
    ) -> Option<String>;
}

/// A destination array that stores and writes to Destinations
#[derive(Default, Debug)]
pub struct BasicDestinationArray {
    dests: HashMap<String, Destination>,
}

impl BasicDestinationArray {
    pub fn get(&self, name: &str) -> Option<&Destination> {
        self.dests.get(name)
    }
}

impl DestinationArray for BasicDestinationArray {
    fn create_text(&mut self, name: &str) {
        self.dests.insert(
            name.to_string(),
            Destination::Text(String::with_capacity(256)),
        );
    }

    fn create_bytes(&mut self, name: &str) {
        self.dests
            .insert(name.to_string(), Destination::Bytes(Vec::new()));
    }

    /// Write bytes into the named destination
    fn write(
        &mut self,
        name: &str,
        bytes: &[u8],
        encoding: Option<&'static encoding_rs::Encoding>,
    ) {
        if let Some(dest) = self.dests.get_mut(name) {
            match dest {
                Destination::Text(_) => {
                    if let Some(decoder) = encoding {
                        let text = &decoder.decode(bytes).0;
                        dest.append_text(text);
                    } else {
                        todo!();
                    }
                }
                Destination::Bytes(_) => {
                    dest.append_bytes(bytes);
                }
            }
        }
    }

    /// Read text from named destination
    fn read_text(
        &self,
        name: &str,
        encoding: Option<&'static encoding_rs::Encoding>,
    ) -> Option<String> {
        self.dests.get(name).and_then(|dest| match dbg!(dest) {
            Destination::Text(s) => Some(s.clone()),
            Destination::Bytes(bs) => encoding.map(|enc| enc.decode(bs).0.to_string()),
        })
    }

    fn destinations(&self) -> Vec<String> {
        self.dests.keys().cloned().collect()
    }
}

/// A destination array that stores rtf lines in a queue from which
/// they can be popped
pub struct RtfQueueDestinationArray {
    basic: BasicDestinationArray,
    queue: VecDeque<String>,
    current: String,
}

impl RtfQueueDestinationArray {
    /// Wrap a basic array with special handling for the "rtf" destination
    pub fn new(basic: BasicDestinationArray) -> Self {
        RtfQueueDestinationArray {
            basic,
            queue: VecDeque::new(),
            current: String::new(),
        }
    }

    /// Pop a line from the front of the queue
    pub fn pop(&mut self) -> Option<String> {
        self.queue.pop_front()
    }

    /// Flush any final content out
    pub fn flush(&mut self) -> Option<String> {
        if !self.current.is_empty() {
            Some(self.current.split_off(0))
        } else {
            None
        }
    }
}

impl DestinationArray for RtfQueueDestinationArray {
    fn destinations(&self) -> Vec<String> {
        self.basic.destinations()
    }

    fn create_text(&mut self, name: &str) {
        if name != "rtf" {
            self.basic.create_text(name);
        }
    }

    fn create_bytes(&mut self, name: &str) {
        if name != "rtf" {
            self.basic.create_bytes(name);
        }
    }

    /// Write bytes into the named destination
    ///
    /// If the destination is "rtf" the incoming text is split into
    /// lines and placed on the queue for retrieval
    fn write(
        &mut self,
        name: &str,
        bytes: &[u8],
        encoding: Option<&'static encoding_rs::Encoding>,
    ) {
        if name == "rtf" {
            if let Some(decoder) = encoding {
                let text = &decoder.decode(bytes).0;
                if text == "\n" {
                    self.queue.push_back(self.current.split_off(0));
                } else {
                    self.current.push_str(text);
                }
            } else {
                panic!("No decoder set");
            }
        } else {
            self.basic.write(name, bytes, encoding);
        }
    }

    fn read_text(
        &self,
        name: &str,
        encoding: Option<&'static encoding_rs::Encoding>,
    ) -> Option<String> {
        self.basic.read_text(name, encoding)
    }
}

/// The engine which is fed tokens and polled for snippets
pub struct SnippetEngine {
    queue: VecDeque<String>,
    dests: Rc<RefCell<dyn DestinationArray>>,
    group_stack: Vec<Group>,
}

impl Default for SnippetEngine {
    fn default() -> Self {
        SnippetEngine {
            queue: VecDeque::new(),
            dests: Rc::new(RefCell::new(BasicDestinationArray::default())),
            group_stack: Vec::new(),
        }
    }
}

impl SnippetEngine {
    pub fn new(destination_array: Rc<RefCell<dyn DestinationArray>>) -> Self {
        SnippetEngine {
            queue: VecDeque::new(),
            dests: destination_array.clone(),
            group_stack: Vec::new(),
        }
    }

    pub fn feed(&mut self, token: &Token) {
        self.consume_token(token);
    }

    pub fn pop(&mut self) -> Option<String> {
        self.queue.pop_back()
    }

    /// Handle a control symbol
    fn do_control_symbol(&mut self, symbol: char, word_is_optional: bool) {
        let mut sym_bytes = [0; 4];
        let sym_str = symbol.encode_utf8(&mut sym_bytes);

        if let Some(group) = self.group_stack.last_mut() {
            if let Some(handler) = SYMBOLS.get(sym_str) {
                handler(group, sym_str, None);
            } else if !word_is_optional {
                // TODO: error
            }
        }
    }

    /// Handle a control word
    fn do_control_word(&mut self, name: &str, arg: Option<i32>, word_is_optional: bool) {
        if let Some(group) = self.group_stack.last_mut() {
            if let Some(handler) = handler(name) {
                handler(group, name, arg);
            } else if !word_is_optional {
                // TODO: error
            }
        }
    }

    /// Write bytes to current top group
    fn write(&mut self, bytes: &[u8]) {
        if let Some(top) = self.group_stack.last_mut() {
            top.write(bytes, None);
        }
    }

    /// Open a new group
    fn open_group(&mut self) {
        let new_group = if let Some(top) = self.group_stack.last() {
            top.clone()
        } else {
            Group::new(self.dests.clone())
        };

        self.group_stack.push(new_group);
    }

    /// Close top group
    fn close_group(&mut self) {
        // if a field result destination has been populated, we pass
        // that text to the parent group
        if let Some(top) = self.group_stack.pop() {
            dbg!(top.array.borrow().destinations());
            if let Some(text) = top.read_text("fldrslt") {
                if let Some(enc) = top.current_encoding {
                    self.write(&enc.encode(text.as_str()).0);
                } else {
                    self.write(text.as_bytes());
                }
            }
        }
    }

    /// Consume a token
    fn consume_token(&mut self, token: &Token) {
        let optional = self
            .group_stack
            .last_mut()
            .map(|g| g.take_ignore_next())
            .unwrap_or(false);

        // Update state for this token
        match token {
            Token::ControlSymbol(c) => self.do_control_symbol(*c, optional),
            Token::ControlWord { name, arg } => self.do_control_word(name, *arg, optional),
            Token::Text(bytes) => self.write(bytes),
            Token::StartGroup => self.open_group(),
            Token::EndGroup => self.close_group(),
            _ => (),
        }
    }
}

/// State of a currently open group
#[derive(Clone)]
pub struct Group {
    /// Array of desintations to write to
    array: Rc<RefCell<dyn DestinationArray>>,
    /// Currently active destination
    current_destination: Option<String>,
    /// Currently specified charset encoding
    current_encoding: Option<&'static encoding_rs::Encoding>,
    /// Values (propagated to child groups by clone)
    values: HashMap<String, Option<i32>>,
    /// Set to make next control optional
    ignore_next_control: bool,
}

impl Group {
    /// Create a new group forwarding writes to the provided DestinationArray
    pub fn new(array: Rc<RefCell<dyn DestinationArray>>) -> Self {
        Group {
            array: array.clone(),
            current_destination: None,
            current_encoding: None,
            values: HashMap::new(),
            ignore_next_control: false,
        }
    }

    /// Set (or clear) a value
    pub fn set_value(&mut self, name: &str, value: Option<i32>) {
        self.values.insert(name.to_string(), value);
    }

    /// Set the current encoding
    pub fn set_encoding(&mut self, encoding: Option<&'static encoding_rs::Encoding>) {
        self.current_encoding = encoding;
    }

    /// Get the current encoding
    pub fn encoding(&self) -> Option<&'static encoding_rs::Encoding> {
        self.current_encoding
    }

    /// Set the current encoding to a codepage
    pub fn set_codepage(&mut self, cp: u16) {
        self.set_encoding(codepage::to_encoding(cp));
    }

    /// Get name of the current destination
    pub fn set_current_destination(&mut self, name: &str) {
        self.current_destination = Some(name.to_string());
    }

    /// Get name of the current destination
    pub fn current_destination(&self) -> Option<&str> {
        self.current_destination.as_deref()
    }

    /// Switch the current destination and create it
    pub fn set_destination(&mut self, name: &str, as_text: bool) {
        self.set_current_destination(name);
        if as_text {
            self.array.borrow_mut().create_text(name);
        } else {
            self.array.borrow_mut().create_bytes(name);
        }
    }

    /// Set the value of the ignore next flag to true
    pub fn set_ignore_next(&mut self) {
        self.ignore_next_control = true;
    }

    /// Take current value of ignore next and clear
    pub fn take_ignore_next(&mut self) -> bool {
        let old = self.ignore_next_control;
        self.ignore_next_control = false;
        old
    }

    /// Write the provided bytes to the current destination
    /// using the current encoding (or override) if required by the
    /// destinationl
    pub fn write(
        &mut self,
        bytes: &[u8],
        override_encoding: Option<&'static encoding_rs::Encoding>,
    ) {
        if let Some(dest) = self.current_destination() {
            self.array.borrow_mut().write(
                dest,
                bytes,
                override_encoding.or_else(|| self.encoding()),
            );
        }
    }

    /// Read text from named destination if posible
    fn read_text(&self, name: &str) -> Option<String> {
        self.array.borrow().read_text(name, self.current_encoding)
    }
}

// RTF_CONTROL
//
// This originally came from compenguy/rtftotext

type StateHandler = dyn Fn(&mut Group, &str, Option<i32>) + 'static + Sync;

lazy_static! {
    // The values for these tables are draw from the Word 2007 RTF Spec (1.9.1)
    // Typically the easiest way to deal with these is to copy/paste the table
    // into a spreadsheet, and filter on the "type" column
    pub static ref DESTINATIONS: HashMap<&'static str, Box<StateHandler>> = {
    let mut m = HashMap::<_, Box<StateHandler>>::new();

    m.insert("aftncn", Box::new(destination_control_set_state_default));
    m.insert("aftnsep", Box::new(destination_control_set_state_default));
    m.insert("aftnsepc", Box::new(destination_control_set_state_default));
    m.insert("annotation", Box::new(destination_control_set_state_default));
    m.insert("atnauthor", Box::new(destination_control_set_state_default));
    m.insert("atndate", Box::new(destination_control_set_state_default));
    m.insert("atnicn", Box::new(destination_control_set_state_default));
    m.insert("atnid", Box::new(destination_control_set_state_default));
    m.insert("atnparent", Box::new(destination_control_set_state_default));
    m.insert("atnref", Box::new(destination_control_set_state_default));
    m.insert("atntime", Box::new(destination_control_set_state_default));
    m.insert("atrfend", Box::new(destination_control_set_state_default));
    m.insert("atrfstart", Box::new(destination_control_set_state_default));
    m.insert("author", Box::new(destination_control_set_state_default));
    m.insert("background", Box::new(destination_control_set_state_default));
    m.insert("bkmkend", Box::new(destination_control_set_state_default));
    m.insert("bkmkstart", Box::new(destination_control_set_state_default));
    m.insert("blipuid", Box::new(destination_control_set_state_default));
    m.insert("buptim", Box::new(destination_control_set_state_default));
    m.insert("category", Box::new(destination_control_set_state_default));
    m.insert("colorschememapping", Box::new(destination_control_set_state_default));
    m.insert("colortbl", Box::new(destination_control_set_state_default));
    m.insert("comment", Box::new(destination_control_set_state_default));
    m.insert("company", Box::new(destination_control_set_state_default));
    m.insert("creatim", Box::new(destination_control_set_state_default));
    m.insert("datafield", Box::new(destination_control_set_state_default));
    m.insert("datastore", Box::new(destination_control_set_state_default));
    m.insert("defchp", Box::new(destination_control_set_state_default));
    m.insert("defpap", Box::new(destination_control_set_state_default));
    m.insert("do", Box::new(destination_control_set_state_default));
    m.insert("doccomm", Box::new(destination_control_set_state_default));
    m.insert("docvar", Box::new(destination_control_set_state_default));
    m.insert("dptxbxtext", Box::new(destination_control_set_state_default));
    m.insert("ebcend", Box::new(destination_control_set_state_default));
    m.insert("ebcstart", Box::new(destination_control_set_state_default));
    m.insert("factoidname", Box::new(destination_control_set_state_default));
    m.insert("falt", Box::new(destination_control_set_state_default));
    m.insert("fchars", Box::new(destination_control_set_state_default));
    m.insert("ffdeftext", Box::new(destination_control_set_state_default));
    m.insert("ffentrymcr", Box::new(destination_control_set_state_default));
    m.insert("ffexitmcr", Box::new(destination_control_set_state_default));
    m.insert("ffformat", Box::new(destination_control_set_state_default));
    m.insert("ffhelptext", Box::new(destination_control_set_state_default));
    m.insert("ffl", Box::new(destination_control_set_state_default));
    m.insert("ffname", Box::new(destination_control_set_state_default));
    m.insert("ffstattext", Box::new(destination_control_set_state_default));
    m.insert("field", Box::new(destination_control_set_state_default));
    m.insert("file", Box::new(destination_control_set_state_default));
    m.insert("filetbl", Box::new(destination_control_set_state_default));
    m.insert("fldinst", Box::new(destination_control_set_state_default));
    m.insert("fldrslt", Box::new(destination_control_set_state_default));
    m.insert("fldtype", Box::new(destination_control_set_state_default));
    m.insert("fname", Box::new(destination_control_set_state_default));
    m.insert("fontemb", Box::new(destination_control_set_state_default));
    m.insert("fontfile", Box::new(destination_control_set_state_default));
    m.insert("fonttbl", Box::new(destination_control_set_state_default));
    m.insert("footer", Box::new(destination_control_set_state_default));
    m.insert("footerf", Box::new(destination_control_set_state_default));
    m.insert("footerl", Box::new(destination_control_set_state_default));
    m.insert("footerr", Box::new(destination_control_set_state_default));
    m.insert("footnote", Box::new(destination_control_set_state_default));
    m.insert("formfield", Box::new(destination_control_set_state_default));
    m.insert("ftncn", Box::new(destination_control_set_state_default));
    m.insert("ftnsep", Box::new(destination_control_set_state_default));
    m.insert("ftnsepc", Box::new(destination_control_set_state_default));
    m.insert("g", Box::new(destination_control_set_state_default));
    m.insert("generator", Box::new(destination_control_set_state_default));
    m.insert("gridtbl", Box::new(destination_control_set_state_default));
    m.insert("header", Box::new(destination_control_set_state_default));
    m.insert("headerf", Box::new(destination_control_set_state_default));
    m.insert("headerl", Box::new(destination_control_set_state_default));
    m.insert("headerr", Box::new(destination_control_set_state_default));
    m.insert("hl", Box::new(destination_control_set_state_default));
    m.insert("hlfr", Box::new(destination_control_set_state_default));
    m.insert("hlinkbase", Box::new(destination_control_set_state_default));
    m.insert("hlloc", Box::new(destination_control_set_state_default));
    m.insert("hlsrc", Box::new(destination_control_set_state_default));
    m.insert("hsv", Box::new(destination_control_set_state_default));
    m.insert("htmltag", Box::new(destination_control_set_state_default));
    m.insert("info", Box::new(destination_control_set_state_default));
    m.insert("keycode", Box::new(destination_control_set_state_default));
    m.insert("keywords", Box::new(destination_control_set_state_default));
    m.insert("latentstyles", Box::new(destination_control_set_state_default));
    m.insert("lchars", Box::new(destination_control_set_state_default));
    m.insert("levelnumbers", Box::new(destination_control_set_state_default));
    m.insert("leveltext", Box::new(destination_control_set_state_default));
    m.insert("lfolevel", Box::new(destination_control_set_state_default));
    m.insert("linkval", Box::new(destination_control_set_state_default));
    m.insert("list", Box::new(destination_control_set_state_default));
    m.insert("listlevel", Box::new(destination_control_set_state_default));
    m.insert("listname", Box::new(destination_control_set_state_default));
    m.insert("listoverride", Box::new(destination_control_set_state_default));
    m.insert("listoverridetable", Box::new(destination_control_set_state_default));
    m.insert("listpicture", Box::new(destination_control_set_state_default));
    m.insert("liststylename", Box::new(destination_control_set_state_default));
    m.insert("listtable", Box::new(destination_control_set_state_default));
    m.insert("listtext", Box::new(destination_control_set_state_default));
    m.insert("lsdlockedexcept", Box::new(destination_control_set_state_default));
    m.insert("macc", Box::new(destination_control_set_state_default));
    m.insert("maccPr", Box::new(destination_control_set_state_default));
    m.insert("mailmerge", Box::new(destination_control_set_state_default));
    m.insert("maln", Box::new(destination_control_set_state_default));
    m.insert("malnScr", Box::new(destination_control_set_state_default));
    m.insert("manager", Box::new(destination_control_set_state_default));
    m.insert("margPr", Box::new(destination_control_set_state_default));
    m.insert("mbar", Box::new(destination_control_set_state_default));
    m.insert("mbarPr", Box::new(destination_control_set_state_default));
    m.insert("mbaseJc", Box::new(destination_control_set_state_default));
    m.insert("mbegChr", Box::new(destination_control_set_state_default));
    m.insert("mborderBox", Box::new(destination_control_set_state_default));
    m.insert("mborderBoxPr", Box::new(destination_control_set_state_default));
    m.insert("mbox", Box::new(destination_control_set_state_default));
    m.insert("mboxPr", Box::new(destination_control_set_state_default));
    m.insert("mchr", Box::new(destination_control_set_state_default));
    m.insert("mcount", Box::new(destination_control_set_state_default));
    m.insert("mctrlPr", Box::new(destination_control_set_state_default));
    m.insert("md", Box::new(destination_control_set_state_default));
    m.insert("mdeg", Box::new(destination_control_set_state_default));
    m.insert("mdegHide", Box::new(destination_control_set_state_default));
    m.insert("mden", Box::new(destination_control_set_state_default));
    m.insert("mdiff", Box::new(destination_control_set_state_default));
    m.insert("mdPr", Box::new(destination_control_set_state_default));
    m.insert("me", Box::new(destination_control_set_state_default));
    m.insert("mendChr", Box::new(destination_control_set_state_default));
    m.insert("meqArr", Box::new(destination_control_set_state_default));
    m.insert("meqArrPr", Box::new(destination_control_set_state_default));
    m.insert("mf", Box::new(destination_control_set_state_default));
    m.insert("mfName", Box::new(destination_control_set_state_default));
    m.insert("mfPr", Box::new(destination_control_set_state_default));
    m.insert("mfunc", Box::new(destination_control_set_state_default));
    m.insert("mfuncPr", Box::new(destination_control_set_state_default));
    m.insert("mgroupChr", Box::new(destination_control_set_state_default));
    m.insert("mgroupChrPr", Box::new(destination_control_set_state_default));
    m.insert("mgrow", Box::new(destination_control_set_state_default));
    m.insert("mhideBot", Box::new(destination_control_set_state_default));
    m.insert("mhideLeft", Box::new(destination_control_set_state_default));
    m.insert("mhideRight", Box::new(destination_control_set_state_default));
    m.insert("mhideTop", Box::new(destination_control_set_state_default));
    m.insert("mhtmltag", Box::new(destination_control_set_state_default));
    m.insert("mlim", Box::new(destination_control_set_state_default));
    m.insert("mlimloc", Box::new(destination_control_set_state_default));
    m.insert("mlimlow", Box::new(destination_control_set_state_default));
    m.insert("mlimlowPr", Box::new(destination_control_set_state_default));
    m.insert("mlimupp", Box::new(destination_control_set_state_default));
    m.insert("mlimuppPr", Box::new(destination_control_set_state_default));
    m.insert("mm", Box::new(destination_control_set_state_default));
    m.insert("mmaddfieldname", Box::new(destination_control_set_state_default));
    m.insert("mmath", Box::new(destination_control_set_state_default));
    m.insert("mmathPict", Box::new(destination_control_set_state_default));
    m.insert("mmathPr", Box::new(destination_control_set_state_default));
    m.insert("mmaxdist", Box::new(destination_control_set_state_default));
    m.insert("mmc", Box::new(destination_control_set_state_default));
    m.insert("mmcJc", Box::new(destination_control_set_state_default));
    m.insert("mmconnectstr", Box::new(destination_control_set_state_default));
    m.insert("mmconnectstrdata", Box::new(destination_control_set_state_default));
    m.insert("mmcPr", Box::new(destination_control_set_state_default));
    m.insert("mmcs", Box::new(destination_control_set_state_default));
    m.insert("mmdatasource", Box::new(destination_control_set_state_default));
    m.insert("mmheadersource", Box::new(destination_control_set_state_default));
    m.insert("mmmailsubject", Box::new(destination_control_set_state_default));
    m.insert("mmodso", Box::new(destination_control_set_state_default));
    m.insert("mmodsofilter", Box::new(destination_control_set_state_default));
    m.insert("mmodsofldmpdata", Box::new(destination_control_set_state_default));
    m.insert("mmodsomappedname", Box::new(destination_control_set_state_default));
    m.insert("mmodsoname", Box::new(destination_control_set_state_default));
    m.insert("mmodsorecipdata", Box::new(destination_control_set_state_default));
    m.insert("mmodsosort", Box::new(destination_control_set_state_default));
    m.insert("mmodsosrc", Box::new(destination_control_set_state_default));
    m.insert("mmodsotable", Box::new(destination_control_set_state_default));
    m.insert("mmodsoudl", Box::new(destination_control_set_state_default));
    m.insert("mmodsoudldata", Box::new(destination_control_set_state_default));
    m.insert("mmodsouniquetag", Box::new(destination_control_set_state_default));
    m.insert("mmPr", Box::new(destination_control_set_state_default));
    m.insert("mmquery", Box::new(destination_control_set_state_default));
    m.insert("mmr", Box::new(destination_control_set_state_default));
    m.insert("mnary", Box::new(destination_control_set_state_default));
    m.insert("mnaryPr", Box::new(destination_control_set_state_default));
    m.insert("mnoBreak", Box::new(destination_control_set_state_default));
    m.insert("mnum", Box::new(destination_control_set_state_default));
    m.insert("mobjDist", Box::new(destination_control_set_state_default));
    m.insert("moMath", Box::new(destination_control_set_state_default));
    m.insert("moMathPara", Box::new(destination_control_set_state_default));
    m.insert("moMathParaPr", Box::new(destination_control_set_state_default));
    m.insert("mopEmu", Box::new(destination_control_set_state_default));
    m.insert("mphant", Box::new(destination_control_set_state_default));
    m.insert("mphantPr", Box::new(destination_control_set_state_default));
    m.insert("mplcHide", Box::new(destination_control_set_state_default));
    m.insert("mpos", Box::new(destination_control_set_state_default));
    m.insert("mr", Box::new(destination_control_set_state_default));
    m.insert("mrad", Box::new(destination_control_set_state_default));
    m.insert("mradPr", Box::new(destination_control_set_state_default));
    m.insert("mrPr", Box::new(destination_control_set_state_default));
    m.insert("msepChr", Box::new(destination_control_set_state_default));
    m.insert("mshow", Box::new(destination_control_set_state_default));
    m.insert("mshp", Box::new(destination_control_set_state_default));
    m.insert("msPre", Box::new(destination_control_set_state_default));
    m.insert("msPrePr", Box::new(destination_control_set_state_default));
    m.insert("msSub", Box::new(destination_control_set_state_default));
    m.insert("msSubPr", Box::new(destination_control_set_state_default));
    m.insert("msSubSup", Box::new(destination_control_set_state_default));
    m.insert("msSubSupPr", Box::new(destination_control_set_state_default));
    m.insert("msSup", Box::new(destination_control_set_state_default));
    m.insert("msSupPr", Box::new(destination_control_set_state_default));
    m.insert("mstrikeBLTR", Box::new(destination_control_set_state_default));
    m.insert("mstrikeH", Box::new(destination_control_set_state_default));
    m.insert("mstrikeTLBR", Box::new(destination_control_set_state_default));
    m.insert("mstrikeV", Box::new(destination_control_set_state_default));
    m.insert("msub", Box::new(destination_control_set_state_default));
    m.insert("msubHide", Box::new(destination_control_set_state_default));
    m.insert("msup", Box::new(destination_control_set_state_default));
    m.insert("msupHide", Box::new(destination_control_set_state_default));
    m.insert("mtransp", Box::new(destination_control_set_state_default));
    m.insert("mtype", Box::new(destination_control_set_state_default));
    m.insert("mvertJc", Box::new(destination_control_set_state_default));
    m.insert("mvfmf", Box::new(destination_control_set_state_default));
    m.insert("mvfml", Box::new(destination_control_set_state_default));
    m.insert("mvtof", Box::new(destination_control_set_state_default));
    m.insert("mvtol", Box::new(destination_control_set_state_default));
    m.insert("mzeroAsc", Box::new(destination_control_set_state_default));
    m.insert("mzeroDesc", Box::new(destination_control_set_state_default));
    m.insert("mzeroWid", Box::new(destination_control_set_state_default));
    m.insert("nesttableprops", Box::new(destination_control_set_state_default));
    m.insert("nextfile", Box::new(destination_control_set_state_default));
    m.insert("nonesttables", Box::new(destination_control_set_state_default));
    m.insert("objalias", Box::new(destination_control_set_state_default));
    m.insert("objclass", Box::new(destination_control_set_state_default));
    m.insert("objdata", Box::new(destination_control_set_state_default));
    m.insert("object", Box::new(destination_control_set_state_default));
    m.insert("objname", Box::new(destination_control_set_state_default));
    m.insert("objsect", Box::new(destination_control_set_state_default));
    m.insert("objtime", Box::new(destination_control_set_state_default));
    m.insert("oldcprops", Box::new(destination_control_set_state_default));
    m.insert("oldpprops", Box::new(destination_control_set_state_default));
    m.insert("oldsprops", Box::new(destination_control_set_state_default));
    m.insert("oldtprops", Box::new(destination_control_set_state_default));
    m.insert("oleclsid", Box::new(destination_control_set_state_default));
    m.insert("operator", Box::new(destination_control_set_state_default));
    m.insert("panose", Box::new(destination_control_set_state_default));
    m.insert("password", Box::new(destination_control_set_state_default));
    m.insert("passwordhash", Box::new(destination_control_set_state_default));
    m.insert("pgp", Box::new(destination_control_set_state_default));
    m.insert("pgptbl", Box::new(destination_control_set_state_default));
    m.insert("picprop", Box::new(destination_control_set_state_default));
    m.insert("pict", Box::new(destination_control_set_state_default));
    m.insert("pn", Box::new(destination_control_set_state_default));
    m.insert("pnseclvl", Box::new(destination_control_and_value_set_state_default));
    // Don't update the current destination, so that the contents of the pntext block get
    // written to the up-level destination, since we don't parse list tables, this serves as an
    // alternate representation
    m.insert("pntext", Box::new(control_word_ignore));
    m.insert("pntxta", Box::new(destination_control_set_state_default));
    m.insert("pntxtb", Box::new(destination_control_set_state_default));
    m.insert("printim", Box::new(destination_control_set_state_default));
    m.insert("private", Box::new(destination_control_set_state_default));
    m.insert("propname", Box::new(destination_control_set_state_default));
    m.insert("protend", Box::new(destination_control_set_state_default));
    m.insert("protstart", Box::new(destination_control_set_state_default));
    m.insert("protusertbl", Box::new(destination_control_set_state_default));
    m.insert("pxe", Box::new(destination_control_set_state_default));
    m.insert("result", Box::new(destination_control_set_state_default));
    m.insert("revtbl", Box::new(destination_control_set_state_default));
    m.insert("revtim", Box::new(destination_control_set_state_default));
    m.insert("rsidtbl", Box::new(destination_control_set_state_default));
    // This is the basic document text destination
    m.insert("rtf", Box::new(destination_control_set_state_encoding));
    m.insert("rxe", Box::new(destination_control_set_state_default));
    m.insert("shp", Box::new(destination_control_set_state_default));
    m.insert("shpgrp", Box::new(destination_control_set_state_default));
    m.insert("shpinst", Box::new(destination_control_set_state_default));
    m.insert("shppict", Box::new(destination_control_set_state_default));
    m.insert("shprslt", Box::new(destination_control_set_state_default));
    m.insert("shptxt", Box::new(destination_control_set_state_default));
    m.insert("sn", Box::new(destination_control_set_state_default));
    m.insert("sp", Box::new(destination_control_set_state_default));
    m.insert("staticval", Box::new(destination_control_set_state_default));
    m.insert("stylesheet", Box::new(destination_control_set_state_default));
    m.insert("subject", Box::new(destination_control_set_state_default));
    m.insert("sv", Box::new(destination_control_set_state_default));
    m.insert("svb", Box::new(destination_control_set_state_default));
    m.insert("tc", Box::new(destination_control_set_state_default));
    m.insert("template", Box::new(destination_control_set_state_default));
    m.insert("themedata", Box::new(destination_control_set_state_default));
    m.insert("title", Box::new(destination_control_set_state_default));
    m.insert("txe", Box::new(destination_control_set_state_default));
    m.insert("ud", Box::new(destination_control_set_state_default));
    m.insert("upr", Box::new(destination_control_set_state_default));
    m.insert("userprops", Box::new(destination_control_set_state_default));
    m.insert("wgrffmtfilter", Box::new(destination_control_set_state_default));
    m.insert("windowcaption", Box::new(destination_control_set_state_default));
    m.insert("writereservation", Box::new(destination_control_set_state_default));
    m.insert("writereservhash", Box::new(destination_control_set_state_default));
    m.insert("xe", Box::new(destination_control_set_state_default));
    m.insert("xform", Box::new(destination_control_set_state_default));
    m.insert("xmlattrname", Box::new(destination_control_set_state_default));
    m.insert("xmlattrvalue", Box::new(destination_control_set_state_default));
    m.insert("xmlclose", Box::new(destination_control_set_state_default));
    m.insert("xmlname", Box::new(destination_control_set_state_default));
    m.insert("xmlnstbl", Box::new(destination_control_set_state_default));
    m.insert("xmlopen", Box::new(destination_control_set_state_default));
    // These are unofficial destinations used by the macOS CocoaRTF export filter
    // https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/AttributedStrings/Tasks/RTFAndAttrStrings.html
    m.insert("NeXTGraphic", Box::new(destination_control_set_state_default));
    m.insert("glid", Box::new(destination_control_and_value_set_state_default));
    m.insert("levelmarker", Box::new(destination_control_set_state_default));
    // These are unofficial destinations used by OpenOffice RTF export filter
    m.insert("hyphen", Box::new(destination_control_and_value_set_state_default));
    m.insert("pgdsc", Box::new(destination_control_and_value_set_state_default));
    m.insert("pgdscno", Box::new(destination_control_and_value_set_state_default));
    m.insert("pgdsctbl", Box::new(destination_control_set_state_default));
    // Found in scrivener
    m.insert("expandedcolortbl", Box::new(destination_control_set_state_default));
    m
    };

    pub static ref SYMBOLS: HashMap<&'static str, Box<StateHandler>> = {
    let mut m = HashMap::<_, Box<StateHandler>>::new();
    m.insert("'", Box::new(control_symbol_write_ansi_char));
    m.insert("-", Box::new(control_word_ignore));
    m.insert("*", Box::new(control_symbol_next_control_is_optional));
    m.insert(":", Box::new(control_word_ignore));
    m.insert("\\", Box::new(control_symbol_write_ansi_char));
    m.insert("_", Box::new(control_symbol_write_ansi_char));
    m.insert("{", Box::new(control_symbol_write_ansi_char));
    m.insert("|", Box::new(control_word_ignore));
    m.insert("}", Box::new(control_symbol_write_ansi_char));
    m.insert("~", Box::new(control_symbol_write_ansi_char));
    m.insert("bullet", Box::new(control_symbol_write_ansi_char));
    m.insert("cell", Box::new(control_value_set_state_and_write_ansi_char));
    m.insert("chatn", Box::new(control_word_ignore));
    m.insert("chdate", Box::new(control_word_ignore));
    m.insert("chdpa", Box::new(control_word_ignore));
    m.insert("chdpl", Box::new(control_word_ignore));
    m.insert("chftn", Box::new(control_word_ignore));
    m.insert("chftnsep", Box::new(control_word_ignore));
    m.insert("chftnsepc", Box::new(control_word_ignore));
    m.insert("chpgn", Box::new(control_word_ignore));
    m.insert("chtime", Box::new(control_word_ignore));
    m.insert("column", Box::new(control_word_ignore));
    m.insert("emdash", Box::new(control_symbol_write_ansi_char));
    m.insert("emspace", Box::new(control_symbol_write_ansi_char));
    m.insert("endash", Box::new(control_symbol_write_ansi_char));
    m.insert("enspace", Box::new(control_symbol_write_ansi_char));
    m.insert("ldblquote", Box::new(control_symbol_write_ansi_char));
    m.insert("line", Box::new(control_symbol_write_ansi_char));
    m.insert("lquote", Box::new(control_symbol_write_ansi_char));
    m.insert("ltrmark", Box::new(control_word_ignore));
    m.insert("nestcell", Box::new(control_word_ignore));
    m.insert("nestrow", Box::new(control_word_ignore));
    m.insert("page", Box::new(control_symbol_write_ansi_char));
    m.insert("par", Box::new(control_symbol_write_ansi_char));
    m.insert("qmspace", Box::new(control_word_ignore));
    m.insert("rdblquote", Box::new(control_symbol_write_ansi_char));
    m.insert("row", Box::new(control_value_set_state_and_write_ansi_char));
    m.insert("rquote", Box::new(control_symbol_write_ansi_char));
    m.insert("rtlmark", Box::new(control_word_ignore));
    m.insert("sect", Box::new(control_symbol_write_ansi_char));
    m.insert("sectnum", Box::new(control_word_ignore));
    m.insert("tab", Box::new(control_symbol_write_ansi_char));
    m.insert("zwbo", Box::new(control_word_ignore));
    m.insert("zwj", Box::new(control_word_ignore));
    m.insert("zwnbo", Box::new(control_word_ignore));
    m.insert("zwnj", Box::new(control_word_ignore));
    // Referenced by the spec as "old-style escaped quotation marks", but not formally
    // recognized in the tables of symbols
    m.insert("\"", Box::new(control_symbol_write_ansi_char));
    // Not official control symbols, but the spec says to make allowances for them
    m.insert("\n", Box::new(control_symbol_write_ansi_char));
    m.insert("\r", Box::new(control_symbol_write_ansi_char));
    m.insert("\t", Box::new(control_symbol_write_ansi_char));
    m.insert(" ", Box::new(control_symbol_write_ansi_char));
    // Not defined anywhere, but I've seen it used
    m.insert("/", Box::new(control_symbol_write_ansi_char));
    m
    };

    pub static ref FLAGS: HashMap<&'static str, Box<StateHandler>> = {
    let mut m = HashMap::<_, Box<StateHandler>>::new();
    m.insert("abslock", Box::new(control_value_set_state_default));
    m.insert("additive", Box::new(control_value_set_state_default));
    m.insert("adjustright", Box::new(control_value_set_state_default));
    m.insert("aenddoc", Box::new(control_value_set_state_default));
    m.insert("aendnotes", Box::new(control_value_set_state_default));
    m.insert("afelev", Box::new(control_value_set_state_default));
    m.insert("aftnbj", Box::new(control_value_set_state_default));
    m.insert("aftnnalc", Box::new(control_value_set_state_default));
    m.insert("aftnnar", Box::new(control_value_set_state_default));
    m.insert("aftnnauc", Box::new(control_value_set_state_default));
    m.insert("aftnnchi", Box::new(control_value_set_state_default));
    m.insert("aftnnchosung", Box::new(control_value_set_state_default));
    m.insert("aftnncnum", Box::new(control_value_set_state_default));
    m.insert("aftnndbar", Box::new(control_value_set_state_default));
    m.insert("aftnndbnum", Box::new(control_value_set_state_default));
    m.insert("aftnndbnumd", Box::new(control_value_set_state_default));
    m.insert("aftnndbnumk", Box::new(control_value_set_state_default));
    m.insert("aftnndbnumt", Box::new(control_value_set_state_default));
    m.insert("aftnnganada", Box::new(control_value_set_state_default));
    m.insert("aftnngbnum", Box::new(control_value_set_state_default));
    m.insert("aftnngbnumd", Box::new(control_value_set_state_default));
    m.insert("aftnngbnumk", Box::new(control_value_set_state_default));
    m.insert("aftnngbnuml", Box::new(control_value_set_state_default));
    m.insert("aftnnrlc", Box::new(control_value_set_state_default));
    m.insert("aftnnruc", Box::new(control_value_set_state_default));
    m.insert("aftnnzodiac", Box::new(control_value_set_state_default));
    m.insert("aftnnzodiacd", Box::new(control_value_set_state_default));
    m.insert("aftnnzodiacl", Box::new(control_value_set_state_default));
    m.insert("aftnrestart", Box::new(control_value_set_state_default));
    m.insert("aftnrstcont", Box::new(control_value_set_state_default));
    m.insert("aftntj", Box::new(control_value_set_state_default));
    m.insert("allowfieldendsel", Box::new(control_value_set_state_default));
    m.insert("allprot", Box::new(control_value_set_state_default));
    m.insert("alntblind", Box::new(control_value_set_state_default));
    m.insert("alt", Box::new(control_value_set_state_default));
    m.insert("annotprot", Box::new(control_value_set_state_default));
    m.insert("ansi", Box::new(control_flag_set_state_encoding));
    m.insert("ApplyBrkRules", Box::new(control_value_set_state_default));
    m.insert("asianbrkrule", Box::new(control_value_set_state_default));
    m.insert("autofmtoverride", Box::new(control_value_set_state_default));
    m.insert("bdbfhdr", Box::new(control_value_set_state_default));
    m.insert("bdrrlswsix", Box::new(control_value_set_state_default));
    m.insert("bgbdiag", Box::new(control_value_set_state_default));
    m.insert("bgcross", Box::new(control_value_set_state_default));
    m.insert("bgdcross", Box::new(control_value_set_state_default));
    m.insert("bgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("bgdkcross", Box::new(control_value_set_state_default));
    m.insert("bgdkdcross", Box::new(control_value_set_state_default));
    m.insert("bgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("bgdkhoriz", Box::new(control_value_set_state_default));
    m.insert("bgdkvert", Box::new(control_value_set_state_default));
    m.insert("bgfdiag", Box::new(control_value_set_state_default));
    m.insert("bghoriz", Box::new(control_value_set_state_default));
    m.insert("bgvert", Box::new(control_value_set_state_default));
    m.insert("bkmkpub", Box::new(control_value_set_state_default));
    m.insert("bookfold", Box::new(control_value_set_state_default));
    m.insert("bookfoldrev", Box::new(control_value_set_state_default));
    m.insert("box", Box::new(control_value_set_state_default));
    m.insert("brdrb", Box::new(control_value_set_state_default));
    m.insert("brdrbar", Box::new(control_value_set_state_default));
    m.insert("brdrbtw", Box::new(control_value_set_state_default));
    m.insert("brdrdash", Box::new(control_value_set_state_default));
    m.insert("brdrdashd", Box::new(control_value_set_state_default));
    m.insert("brdrdashdd", Box::new(control_value_set_state_default));
    m.insert("brdrdashdot", Box::new(control_value_set_state_default));
    m.insert("brdrdashdotdot", Box::new(control_value_set_state_default));
    m.insert("brdrdashdotstr", Box::new(control_value_set_state_default));
    m.insert("brdrdashsm", Box::new(control_value_set_state_default));
    m.insert("brdrdb", Box::new(control_value_set_state_default));
    m.insert("brdrdot", Box::new(control_value_set_state_default));
    m.insert("brdremboss", Box::new(control_value_set_state_default));
    m.insert("brdrengrave", Box::new(control_value_set_state_default));
    m.insert("brdrframe", Box::new(control_value_set_state_default));
    m.insert("brdrhair", Box::new(control_value_set_state_default));
    m.insert("brdrinset", Box::new(control_value_set_state_default));
    m.insert("brdrl", Box::new(control_value_set_state_default));
    m.insert("brdrnil", Box::new(control_value_set_state_default));
    m.insert("brdrnone", Box::new(control_value_set_state_default));
    m.insert("brdroutset", Box::new(control_value_set_state_default));
    m.insert("brdrr", Box::new(control_value_set_state_default));
    m.insert("brdrs", Box::new(control_value_set_state_default));
    m.insert("brdrsh", Box::new(control_value_set_state_default));
    m.insert("brdrt", Box::new(control_value_set_state_default));
    m.insert("brdrtbl", Box::new(control_value_set_state_default));
    m.insert("brdrth", Box::new(control_value_set_state_default));
    m.insert("brdrthtnlg", Box::new(control_value_set_state_default));
    m.insert("brdrthtnmg", Box::new(control_value_set_state_default));
    m.insert("brdrthtnsg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthlg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthmg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthsg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthtnlg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthtnmg", Box::new(control_value_set_state_default));
    m.insert("brdrtnthtnsg", Box::new(control_value_set_state_default));
    m.insert("brdrtriple", Box::new(control_value_set_state_default));
    m.insert("brdrwavy", Box::new(control_value_set_state_default));
    m.insert("brdrwavydb", Box::new(control_value_set_state_default));
    m.insert("brkfrm", Box::new(control_value_set_state_default));
    m.insert("bxe", Box::new(control_value_set_state_default));
    m.insert("caccentfive", Box::new(control_value_set_state_default));
    m.insert("caccentfour", Box::new(control_value_set_state_default));
    m.insert("caccentone", Box::new(control_value_set_state_default));
    m.insert("caccentsix", Box::new(control_value_set_state_default));
    m.insert("caccentthree", Box::new(control_value_set_state_default));
    m.insert("caccenttwo", Box::new(control_value_set_state_default));
    m.insert("cachedcolbal", Box::new(control_value_set_state_default));
    m.insert("cbackgroundone", Box::new(control_value_set_state_default));
    m.insert("cbackgroundtwo", Box::new(control_value_set_state_default));
    m.insert("cfollowedhyperlink", Box::new(control_value_set_state_default));
    m.insert("chbgbdiag", Box::new(control_value_set_state_default));
    m.insert("chbgcross", Box::new(control_value_set_state_default));
    m.insert("chbgdcross", Box::new(control_value_set_state_default));
    m.insert("chbgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("chbgdkcross", Box::new(control_value_set_state_default));
    m.insert("chbgdkdcross", Box::new(control_value_set_state_default));
    m.insert("chbgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("chbgdkhoriz", Box::new(control_value_set_state_default));
    m.insert("chbgdkvert", Box::new(control_value_set_state_default));
    m.insert("chbgfdiag", Box::new(control_value_set_state_default));
    m.insert("chbghoriz", Box::new(control_value_set_state_default));
    m.insert("chbgvert", Box::new(control_value_set_state_default));
    m.insert("chbrdr", Box::new(control_value_set_state_default));
    m.insert("chyperlink", Box::new(control_value_set_state_default));
    m.insert("clbgbdiag", Box::new(control_value_set_state_default));
    m.insert("clbgcross", Box::new(control_value_set_state_default));
    m.insert("clbgdcross", Box::new(control_value_set_state_default));
    m.insert("clbgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("clbgdkcross", Box::new(control_value_set_state_default));
    m.insert("clbgdkdcross", Box::new(control_value_set_state_default));
    m.insert("clbgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("clbgdkhor", Box::new(control_value_set_state_default));
    m.insert("clbgdkvert", Box::new(control_value_set_state_default));
    m.insert("clbgfdiag", Box::new(control_value_set_state_default));
    m.insert("clbghoriz", Box::new(control_value_set_state_default));
    m.insert("clbgvert", Box::new(control_value_set_state_default));
    m.insert("clbrdrb", Box::new(control_value_set_state_default));
    m.insert("clbrdrl", Box::new(control_value_set_state_default));
    m.insert("clbrdrr", Box::new(control_value_set_state_default));
    m.insert("clbrdrt", Box::new(control_value_set_state_default));
    m.insert("cldel", Box::new(control_value_set_state_default));
    m.insert("cldgll", Box::new(control_value_set_state_default));
    m.insert("cldglu", Box::new(control_value_set_state_default));
    m.insert("clFitText", Box::new(control_value_set_state_default));
    m.insert("clhidemark", Box::new(control_value_set_state_default));
    m.insert("clins", Box::new(control_value_set_state_default));
    m.insert("clmgf", Box::new(control_value_set_state_default));
    m.insert("clmrg", Box::new(control_value_set_state_default));
    m.insert("clmrgd", Box::new(control_value_set_state_default));
    m.insert("clmrgdr", Box::new(control_value_set_state_default));
    m.insert("clNoWrap", Box::new(control_value_set_state_default));
    m.insert("clshdrawnil", Box::new(control_value_set_state_default));
    m.insert("clsplit", Box::new(control_value_set_state_default));
    m.insert("clsplitr", Box::new(control_value_set_state_default));
    m.insert("cltxbtlr", Box::new(control_value_set_state_default));
    m.insert("cltxlrtb", Box::new(control_value_set_state_default));
    m.insert("cltxlrtbv", Box::new(control_value_set_state_default));
    m.insert("cltxtbrl", Box::new(control_value_set_state_default));
    m.insert("cltxtbrlv", Box::new(control_value_set_state_default));
    m.insert("clvertalb", Box::new(control_value_set_state_default));
    m.insert("clvertalc", Box::new(control_value_set_state_default));
    m.insert("clvertalt", Box::new(control_value_set_state_default));
    m.insert("clvmgf", Box::new(control_value_set_state_default));
    m.insert("clvmrg", Box::new(control_value_set_state_default));
    m.insert("cmaindarkone", Box::new(control_value_set_state_default));
    m.insert("cmaindarktwo", Box::new(control_value_set_state_default));
    m.insert("cmainlightone", Box::new(control_value_set_state_default));
    m.insert("cmainlighttwo", Box::new(control_value_set_state_default));
    m.insert("collapsed", Box::new(control_value_set_state_default));
    m.insert("contextualspace", Box::new(control_value_set_state_default));
    m.insert("ctextone", Box::new(control_value_set_state_default));
    m.insert("ctexttwo", Box::new(control_value_set_state_default));
    m.insert("ctrl", Box::new(control_value_set_state_default));
    m.insert("cvmme", Box::new(control_value_set_state_default));
    m.insert("date", Box::new(control_value_set_state_default));
    m.insert("dbch", Box::new(control_value_set_state_default));
    m.insert("defformat", Box::new(control_value_set_state_default));
    m.insert("defshp", Box::new(control_value_set_state_default));
    m.insert("dgmargin", Box::new(control_value_set_state_default));
    m.insert("dgsnap", Box::new(control_value_set_state_default));
    m.insert("dntblnsbdb", Box::new(control_value_set_state_default));
    m.insert("dobxcolumn", Box::new(control_value_set_state_default));
    m.insert("dobxmargin", Box::new(control_value_set_state_default));
    m.insert("dobxpage", Box::new(control_value_set_state_default));
    m.insert("dobymargin", Box::new(control_value_set_state_default));
    m.insert("dobypage", Box::new(control_value_set_state_default));
    m.insert("dobypara", Box::new(control_value_set_state_default));
    m.insert("doctemp", Box::new(control_value_set_state_default));
    m.insert("dolock", Box::new(control_value_set_state_default));
    m.insert("donotshowcomments", Box::new(control_value_set_state_default));
    m.insert("donotshowinsdel", Box::new(control_value_set_state_default));
    m.insert("donotshowmarkup", Box::new(control_value_set_state_default));
    m.insert("donotshowprops", Box::new(control_value_set_state_default));
    m.insert("dpaendhol", Box::new(control_value_set_state_default));
    m.insert("dpaendsol", Box::new(control_value_set_state_default));
    m.insert("dparc", Box::new(control_value_set_state_default));
    m.insert("dparcflipx", Box::new(control_value_set_state_default));
    m.insert("dparcflipy", Box::new(control_value_set_state_default));
    m.insert("dpastarthol", Box::new(control_value_set_state_default));
    m.insert("dpastartsol", Box::new(control_value_set_state_default));
    m.insert("dpcallout", Box::new(control_value_set_state_default));
    m.insert("dpcoaccent", Box::new(control_value_set_state_default));
    m.insert("dpcobestfit", Box::new(control_value_set_state_default));
    m.insert("dpcoborder", Box::new(control_value_set_state_default));
    m.insert("dpcodabs", Box::new(control_value_set_state_default));
    m.insert("dpcodbottom", Box::new(control_value_set_state_default));
    m.insert("dpcodcenter", Box::new(control_value_set_state_default));
    m.insert("dpcodtop", Box::new(control_value_set_state_default));
    m.insert("dpcominusx", Box::new(control_value_set_state_default));
    m.insert("dpcominusy", Box::new(control_value_set_state_default));
    m.insert("dpcosmarta", Box::new(control_value_set_state_default));
    m.insert("dpcotdouble", Box::new(control_value_set_state_default));
    m.insert("dpcotright", Box::new(control_value_set_state_default));
    m.insert("dpcotsingle", Box::new(control_value_set_state_default));
    m.insert("dpcottriple", Box::new(control_value_set_state_default));
    m.insert("dpellipse", Box::new(control_value_set_state_default));
    m.insert("dpendgroup", Box::new(control_value_set_state_default));
    m.insert("dpfillbgpal", Box::new(control_value_set_state_default));
    m.insert("dpfillfgpal", Box::new(control_value_set_state_default));
    m.insert("dpgroup", Box::new(control_value_set_state_default));
    m.insert("dpline", Box::new(control_value_set_state_default));
    m.insert("dplinedado", Box::new(control_value_set_state_default));
    m.insert("dplinedadodo", Box::new(control_value_set_state_default));
    m.insert("dplinedash", Box::new(control_value_set_state_default));
    m.insert("dplinedot", Box::new(control_value_set_state_default));
    m.insert("dplinehollow", Box::new(control_value_set_state_default));
    m.insert("dplinepal", Box::new(control_value_set_state_default));
    m.insert("dplinesolid", Box::new(control_value_set_state_default));
    m.insert("dppolygon", Box::new(control_value_set_state_default));
    m.insert("dppolyline", Box::new(control_value_set_state_default));
    m.insert("dprect", Box::new(control_value_set_state_default));
    m.insert("dproundr", Box::new(control_value_set_state_default));
    m.insert("dpshadow", Box::new(control_value_set_state_default));
    m.insert("dptxbtlr", Box::new(control_value_set_state_default));
    m.insert("dptxbx", Box::new(control_value_set_state_default));
    m.insert("dptxlrtb", Box::new(control_value_set_state_default));
    m.insert("dptxlrtbv", Box::new(control_value_set_state_default));
    m.insert("dptxtbrl", Box::new(control_value_set_state_default));
    m.insert("dptxtbrlv", Box::new(control_value_set_state_default));
    m.insert("emfblip", Box::new(control_value_set_state_default));
    m.insert("enddoc", Box::new(control_value_set_state_default));
    m.insert("endnhere", Box::new(control_value_set_state_default));
    m.insert("endnotes", Box::new(control_value_set_state_default));
    m.insert("expshrtn", Box::new(control_value_set_state_default));
    m.insert("faauto", Box::new(control_value_set_state_default));
    m.insert("facenter", Box::new(control_value_set_state_default));
    m.insert("facingp", Box::new(control_value_set_state_default));
    m.insert("fafixed", Box::new(control_value_set_state_default));
    m.insert("fahang", Box::new(control_value_set_state_default));
    m.insert("faroman", Box::new(control_value_set_state_default));
    m.insert("favar", Box::new(control_value_set_state_default));
    m.insert("fbidi", Box::new(control_value_set_state_default));
    m.insert("fbidis", Box::new(control_value_set_state_default));
    m.insert("fbimajor", Box::new(control_value_set_state_default));
    m.insert("fbiminor", Box::new(control_value_set_state_default));
    m.insert("fdbmajor", Box::new(control_value_set_state_default));
    m.insert("fdbminor", Box::new(control_value_set_state_default));
    m.insert("fdecor", Box::new(control_value_set_state_default));
    m.insert("felnbrelev", Box::new(control_value_set_state_default));
    m.insert("fetch", Box::new(control_value_set_state_default));
    m.insert("fhimajor", Box::new(control_value_set_state_default));
    m.insert("fhiminor", Box::new(control_value_set_state_default));
    m.insert("fjgothic", Box::new(control_value_set_state_default));
    m.insert("fjminchou", Box::new(control_value_set_state_default));
    m.insert("fldalt", Box::new(control_value_set_state_default));
    m.insert("flddirty", Box::new(control_value_set_state_default));
    m.insert("fldedit", Box::new(control_value_set_state_default));
    m.insert("fldlock", Box::new(control_value_set_state_default));
    m.insert("fldpriv", Box::new(control_value_set_state_default));
    m.insert("flomajor", Box::new(control_value_set_state_default));
    m.insert("flominor", Box::new(control_value_set_state_default));
    m.insert("fmodern", Box::new(control_value_set_state_default));
    m.insert("fnetwork", Box::new(control_value_set_state_default));
    m.insert("fnil", Box::new(control_value_set_state_default));
    m.insert("fnonfilesys", Box::new(control_value_set_state_default));
    m.insert("forceupgrade", Box::new(control_value_set_state_default));
    m.insert("formdisp", Box::new(control_value_set_state_default));
    m.insert("formprot", Box::new(control_value_set_state_default));
    m.insert("formshade", Box::new(control_value_set_state_default));
    m.insert("fracwidth", Box::new(control_value_set_state_default));
    m.insert("frmtxbtlr", Box::new(control_value_set_state_default));
    m.insert("frmtxlrtb", Box::new(control_value_set_state_default));
    m.insert("frmtxlrtbv", Box::new(control_value_set_state_default));
    m.insert("frmtxtbrl", Box::new(control_value_set_state_default));
    m.insert("frmtxtbrlv", Box::new(control_value_set_state_default));
    m.insert("froman", Box::new(control_value_set_state_default));
    m.insert("fromtext", Box::new(control_value_set_state_default));
    m.insert("fscript", Box::new(control_value_set_state_default));
    m.insert("fswiss", Box::new(control_value_set_state_default));
    m.insert("ftech", Box::new(control_value_set_state_default));
    m.insert("ftnalt", Box::new(control_value_set_state_default));
    m.insert("ftnbj", Box::new(control_value_set_state_default));
    m.insert("ftnil", Box::new(control_value_set_state_default));
    m.insert("ftnlytwnine", Box::new(control_value_set_state_default));
    m.insert("ftnnalc", Box::new(control_value_set_state_default));
    m.insert("ftnnar", Box::new(control_value_set_state_default));
    m.insert("ftnnauc", Box::new(control_value_set_state_default));
    m.insert("ftnnchi", Box::new(control_value_set_state_default));
    m.insert("ftnnchosung", Box::new(control_value_set_state_default));
    m.insert("ftnncnum", Box::new(control_value_set_state_default));
    m.insert("ftnndbar", Box::new(control_value_set_state_default));
    m.insert("ftnndbnum", Box::new(control_value_set_state_default));
    m.insert("ftnndbnumd", Box::new(control_value_set_state_default));
    m.insert("ftnndbnumk", Box::new(control_value_set_state_default));
    m.insert("ftnndbnumt", Box::new(control_value_set_state_default));
    m.insert("ftnnganada", Box::new(control_value_set_state_default));
    m.insert("ftnngbnum", Box::new(control_value_set_state_default));
    m.insert("ftnngbnumd", Box::new(control_value_set_state_default));
    m.insert("ftnngbnumk", Box::new(control_value_set_state_default));
    m.insert("ftnngbnuml", Box::new(control_value_set_state_default));
    m.insert("ftnnrlc", Box::new(control_value_set_state_default));
    m.insert("ftnnruc", Box::new(control_value_set_state_default));
    m.insert("ftnnzodiac", Box::new(control_value_set_state_default));
    m.insert("ftnnzodiacd", Box::new(control_value_set_state_default));
    m.insert("ftnnzodiacl", Box::new(control_value_set_state_default));
    m.insert("ftnrestart", Box::new(control_value_set_state_default));
    m.insert("ftnrstcont", Box::new(control_value_set_state_default));
    m.insert("ftnrstpg", Box::new(control_value_set_state_default));
    m.insert("ftntj", Box::new(control_value_set_state_default));
    m.insert("fttruetype", Box::new(control_value_set_state_default));
    m.insert("fvaliddos", Box::new(control_value_set_state_default));
    m.insert("fvalidhpfs", Box::new(control_value_set_state_default));
    m.insert("fvalidmac", Box::new(control_value_set_state_default));
    m.insert("fvalidntfs", Box::new(control_value_set_state_default));
    m.insert("gutterprl", Box::new(control_value_set_state_default));
    m.insert("hich", Box::new(control_value_set_state_default));
    m.insert("horzdoc", Box::new(control_value_set_state_default));
    m.insert("horzsect", Box::new(control_value_set_state_default));
    m.insert("hrule", Box::new(control_value_set_state_default));
    m.insert("htmautsp", Box::new(control_value_set_state_default));
    m.insert("htmlbase", Box::new(control_value_set_state_default));
    m.insert("hwelev", Box::new(control_value_set_state_default));
    m.insert("indmirror", Box::new(control_value_set_state_default));
    m.insert("indrlsweleven", Box::new(control_value_set_state_default));
    m.insert("intbl", Box::new(control_value_set_state_default));
    m.insert("ixe", Box::new(control_value_set_state_default));
    m.insert("jcompress", Box::new(control_value_set_state_default));
    m.insert("jexpand", Box::new(control_value_set_state_default));
    m.insert("jis", Box::new(control_value_set_state_default));
    m.insert("jpegblip", Box::new(control_value_set_state_default));
    m.insert("jsksu", Box::new(control_value_set_state_default));
    m.insert("keep", Box::new(control_value_set_state_default));
    m.insert("keepn", Box::new(control_value_set_state_default));
    m.insert("krnprsnet", Box::new(control_value_set_state_default));
    m.insert("jclisttab", Box::new(control_value_set_state_default));
    m.insert("landscape", Box::new(control_value_set_state_default));
    m.insert("lastrow", Box::new(control_value_set_state_default));
    m.insert("levelpicturenosize", Box::new(control_value_set_state_default));
    m.insert("linebetcol", Box::new(control_value_set_state_default));
    m.insert("linecont", Box::new(control_value_set_state_default));
    m.insert("lineppage", Box::new(control_value_set_state_default));
    m.insert("linerestart", Box::new(control_value_set_state_default));
    m.insert("linkself", Box::new(control_value_set_state_default));
    m.insert("linkstyles", Box::new(control_value_set_state_default));
    m.insert("listhybrid", Box::new(control_value_set_state_default));
    m.insert("listoverridestartat", Box::new(control_value_set_state_default));
    m.insert("lnbrkrule", Box::new(control_value_set_state_default));
    m.insert("lndscpsxn", Box::new(control_value_set_state_default));
    m.insert("lnongrid", Box::new(control_value_set_state_default));
    m.insert("loch", Box::new(control_value_set_state_default));
    m.insert("ltrch", Box::new(control_value_set_state_default));
    m.insert("ltrdoc", Box::new(control_value_set_state_default));
    m.insert("ltrpar", Box::new(control_value_set_state_default));
    m.insert("ltrrow", Box::new(control_value_set_state_default));
    m.insert("ltrsect", Box::new(control_value_set_state_default));
    m.insert("lvltentative", Box::new(control_value_set_state_default));
    m.insert("lytcalctblwd", Box::new(control_value_set_state_default));
    m.insert("lytexcttp", Box::new(control_value_set_state_default));
    m.insert("lytprtmet", Box::new(control_value_set_state_default));
    m.insert("lyttblrtgr", Box::new(control_value_set_state_default));
    m.insert("mac", Box::new(control_flag_set_state_encoding));
    m.insert("macpict", Box::new(control_value_set_state_default));
    m.insert("makebackup", Box::new(control_value_set_state_default));
    m.insert("margmirror", Box::new(control_value_set_state_default));
    m.insert("margmirsxn", Box::new(control_value_set_state_default));
    m.insert("mlit", Box::new(control_value_set_state_default));
    m.insert("mmattach", Box::new(control_value_set_state_default));
    m.insert("mmblanklines", Box::new(control_value_set_state_default));
    m.insert("mmdatatypeaccess", Box::new(control_value_set_state_default));
    m.insert("mmdatatypeexcel", Box::new(control_value_set_state_default));
    m.insert("mmdatatypefile", Box::new(control_value_set_state_default));
    m.insert("mmdatatypeodbc", Box::new(control_value_set_state_default));
    m.insert("mmdatatypeodso", Box::new(control_value_set_state_default));
    m.insert("mmdatatypeqt", Box::new(control_value_set_state_default));
    m.insert("mmdefaultsql", Box::new(control_value_set_state_default));
    m.insert("mmdestemail", Box::new(control_value_set_state_default));
    m.insert("mmdestfax", Box::new(control_value_set_state_default));
    m.insert("mmdestnewdoc", Box::new(control_value_set_state_default));
    m.insert("mmdestprinter", Box::new(control_value_set_state_default));
    m.insert("mmfttypeaddress", Box::new(control_value_set_state_default));
    m.insert("mmfttypebarcode", Box::new(control_value_set_state_default));
    m.insert("mmfttypedbcolumn", Box::new(control_value_set_state_default));
    m.insert("mmfttypemapped", Box::new(control_value_set_state_default));
    m.insert("mmfttypenull", Box::new(control_value_set_state_default));
    m.insert("mmfttypesalutation", Box::new(control_value_set_state_default));
    m.insert("mmlinktoquery", Box::new(control_value_set_state_default));
    m.insert("mmmaintypecatalog", Box::new(control_value_set_state_default));
    m.insert("mmmaintypeemail", Box::new(control_value_set_state_default));
    m.insert("mmmaintypeenvelopes", Box::new(control_value_set_state_default));
    m.insert("mmmaintypefax", Box::new(control_value_set_state_default));
    m.insert("mmmaintypelabels", Box::new(control_value_set_state_default));
    m.insert("mmmaintypeletters", Box::new(control_value_set_state_default));
    m.insert("mmshowdata", Box::new(control_value_set_state_default));
    m.insert("mnor", Box::new(control_value_set_state_default));
    m.insert("msmcap", Box::new(control_value_set_state_default));
    m.insert("muser", Box::new(control_value_set_state_default));
    m.insert("mvf", Box::new(control_value_set_state_default));
    m.insert("mvt", Box::new(control_value_set_state_default));
    m.insert("newtblstyruls", Box::new(control_value_set_state_default));
    m.insert("noafcnsttbl", Box::new(control_value_set_state_default));
    m.insert("nobrkwrptbl", Box::new(control_value_set_state_default));
    m.insert("nocolbal", Box::new(control_value_set_state_default));
    m.insert("nocompatoptions", Box::new(control_value_set_state_default));
    m.insert("nocwrap", Box::new(control_value_set_state_default));
    m.insert("nocxsptable", Box::new(control_value_set_state_default));
    m.insert("noextrasprl", Box::new(control_value_set_state_default));
    m.insert("nofeaturethrottle", Box::new(control_value_set_state_default));
    m.insert("nogrowautofit", Box::new(control_value_set_state_default));
    m.insert("noindnmbrts", Box::new(control_value_set_state_default));
    m.insert("nojkernpunct", Box::new(control_value_set_state_default));
    m.insert("nolead", Box::new(control_value_set_state_default));
    m.insert("noline", Box::new(control_value_set_state_default));
    m.insert("nolnhtadjtbl", Box::new(control_value_set_state_default));
    m.insert("nonshppict", Box::new(control_value_set_state_default));
    m.insert("nooverflow", Box::new(control_value_set_state_default));
    m.insert("noproof", Box::new(control_value_set_state_default));
    m.insert("noqfpromote", Box::new(control_value_set_state_default));
    m.insert("nosectexpand", Box::new(control_value_set_state_default));
    m.insert("nosnaplinegrid", Box::new(control_value_set_state_default));
    m.insert("nospaceforul", Box::new(control_value_set_state_default));
    m.insert("nosupersub", Box::new(control_value_set_state_default));
    m.insert("notabind", Box::new(control_value_set_state_default));
    m.insert("notbrkcnstfrctbl", Box::new(control_value_set_state_default));
    m.insert("notcvasp", Box::new(control_value_set_state_default));
    m.insert("notvatxbx", Box::new(control_value_set_state_default));
    m.insert("nouicompat", Box::new(control_value_set_state_default));
    m.insert("noultrlspc", Box::new(control_value_set_state_default));
    m.insert("nowidctlpar", Box::new(control_value_set_state_default));
    m.insert("nowrap", Box::new(control_value_set_state_default));
    m.insert("nowwrap", Box::new(control_value_set_state_default));
    m.insert("noxlattoyen", Box::new(control_value_set_state_default));
    m.insert("objattph", Box::new(control_value_set_state_default));
    m.insert("objautlink", Box::new(control_value_set_state_default));
    m.insert("objemb", Box::new(control_value_set_state_default));
    m.insert("objhtml", Box::new(control_value_set_state_default));
    m.insert("objicemb", Box::new(control_value_set_state_default));
    m.insert("objlink", Box::new(control_value_set_state_default));
    m.insert("objlock", Box::new(control_value_set_state_default));
    m.insert("objocx", Box::new(control_value_set_state_default));
    m.insert("objpub", Box::new(control_value_set_state_default));
    m.insert("objsetsize", Box::new(control_value_set_state_default));
    m.insert("objsub", Box::new(control_value_set_state_default));
    m.insert("objupdate", Box::new(control_value_set_state_default));
    m.insert("oldas", Box::new(control_value_set_state_default));
    m.insert("oldlinewrap", Box::new(control_value_set_state_default));
    m.insert("otblrul", Box::new(control_value_set_state_default));
    m.insert("overlay", Box::new(control_value_set_state_default));
    m.insert("pagebb", Box::new(control_value_set_state_default));
    m.insert("pard", Box::new(control_value_set_state_default));
    m.insert("pc", Box::new(control_flag_set_state_encoding));
    m.insert("pca", Box::new(control_flag_set_state_encoding));
    m.insert("pgbrdrb", Box::new(control_value_set_state_default));
    m.insert("pgbrdrfoot", Box::new(control_value_set_state_default));
    m.insert("pgbrdrhead", Box::new(control_value_set_state_default));
    m.insert("pgbrdrl", Box::new(control_value_set_state_default));
    m.insert("pgbrdrr", Box::new(control_value_set_state_default));
    m.insert("pgbrdrsnap", Box::new(control_value_set_state_default));
    m.insert("pgbrdrt", Box::new(control_value_set_state_default));
    m.insert("pgnbidia", Box::new(control_value_set_state_default));
    m.insert("pgnbidib", Box::new(control_value_set_state_default));
    m.insert("pgnchosung", Box::new(control_value_set_state_default));
    m.insert("pgncnum", Box::new(control_value_set_state_default));
    m.insert("pgncont", Box::new(control_value_set_state_default));
    m.insert("pgndbnum", Box::new(control_value_set_state_default));
    m.insert("pgndbnumd", Box::new(control_value_set_state_default));
    m.insert("pgndbnumk", Box::new(control_value_set_state_default));
    m.insert("pgndbnumt", Box::new(control_value_set_state_default));
    m.insert("pgndec", Box::new(control_value_set_state_default));
    m.insert("pgndecd", Box::new(control_value_set_state_default));
    m.insert("pgnganada", Box::new(control_value_set_state_default));
    m.insert("pgngbnum", Box::new(control_value_set_state_default));
    m.insert("pgngbnumd", Box::new(control_value_set_state_default));
    m.insert("pgngbnumk", Box::new(control_value_set_state_default));
    m.insert("pgngbnuml", Box::new(control_value_set_state_default));
    m.insert("pgnhindia", Box::new(control_value_set_state_default));
    m.insert("pgnhindib", Box::new(control_value_set_state_default));
    m.insert("pgnhindic", Box::new(control_value_set_state_default));
    m.insert("pgnhindid", Box::new(control_value_set_state_default));
    m.insert("pgnhnsc", Box::new(control_value_set_state_default));
    m.insert("pgnhnsh", Box::new(control_value_set_state_default));
    m.insert("pgnhnsm", Box::new(control_value_set_state_default));
    m.insert("pgnhnsn", Box::new(control_value_set_state_default));
    m.insert("pgnhnsp", Box::new(control_value_set_state_default));
    m.insert("pgnid", Box::new(control_value_set_state_default));
    m.insert("pgnlcltr", Box::new(control_value_set_state_default));
    m.insert("pgnlcrm", Box::new(control_value_set_state_default));
    m.insert("pgnrestart", Box::new(control_value_set_state_default));
    m.insert("pgnthaia", Box::new(control_value_set_state_default));
    m.insert("pgnthaib", Box::new(control_value_set_state_default));
    m.insert("pgnthaic", Box::new(control_value_set_state_default));
    m.insert("pgnucltr", Box::new(control_value_set_state_default));
    m.insert("pgnucrm", Box::new(control_value_set_state_default));
    m.insert("pgnvieta", Box::new(control_value_set_state_default));
    m.insert("pgnzodiac", Box::new(control_value_set_state_default));
    m.insert("pgnzodiacd", Box::new(control_value_set_state_default));
    m.insert("pgnzodiacl", Box::new(control_value_set_state_default));
    m.insert("phcol", Box::new(control_value_set_state_default));
    m.insert("phmrg", Box::new(control_value_set_state_default));
    m.insert("phpg", Box::new(control_value_set_state_default));
    m.insert("picbmp", Box::new(control_value_set_state_default));
    m.insert("picscaled", Box::new(control_value_set_state_default));
    m.insert("pindtabqc", Box::new(control_value_set_state_default));
    m.insert("pindtabql", Box::new(control_value_set_state_default));
    m.insert("pindtabqr", Box::new(control_value_set_state_default));
    m.insert("plain", Box::new(control_value_set_state_default));
    m.insert("pmartabqc", Box::new(control_value_set_state_default));
    m.insert("pmartabql", Box::new(control_value_set_state_default));
    m.insert("pmartabqr", Box::new(control_value_set_state_default));
    m.insert("pnacross", Box::new(control_value_set_state_default));
    m.insert("pnaiu", Box::new(control_value_set_state_default));
    m.insert("pnaiud", Box::new(control_value_set_state_default));
    m.insert("pnaiueo", Box::new(control_value_set_state_default));
    m.insert("pnaiueod", Box::new(control_value_set_state_default));
    m.insert("pnbidia", Box::new(control_value_set_state_default));
    m.insert("pnbidib", Box::new(control_value_set_state_default));
    m.insert("pncard", Box::new(control_value_set_state_default));
    m.insert("pnchosung", Box::new(control_value_set_state_default));
    m.insert("pncnum", Box::new(control_value_set_state_default));
    m.insert("pndbnum", Box::new(control_value_set_state_default));
    m.insert("pndbnumd", Box::new(control_value_set_state_default));
    m.insert("pndbnumk", Box::new(control_value_set_state_default));
    m.insert("pndbnuml", Box::new(control_value_set_state_default));
    m.insert("pndbnumt", Box::new(control_value_set_state_default));
    m.insert("pndec", Box::new(control_value_set_state_default));
    m.insert("pndecd", Box::new(control_value_set_state_default));
    m.insert("pnganada", Box::new(control_value_set_state_default));
    m.insert("pngblip", Box::new(control_value_set_state_default));
    m.insert("pngbnum", Box::new(control_value_set_state_default));
    m.insert("pngbnumd", Box::new(control_value_set_state_default));
    m.insert("pngbnumk", Box::new(control_value_set_state_default));
    m.insert("pngbnuml", Box::new(control_value_set_state_default));
    m.insert("pnhang", Box::new(control_value_set_state_default));
    m.insert("pniroha", Box::new(control_value_set_state_default));
    m.insert("pnirohad", Box::new(control_value_set_state_default));
    m.insert("pnlcltr", Box::new(control_value_set_state_default));
    m.insert("pnlcrm", Box::new(control_value_set_state_default));
    m.insert("pnlvlblt", Box::new(control_value_set_state_default));
    m.insert("pnlvlbody", Box::new(control_value_set_state_default));
    m.insert("pnlvlcont", Box::new(control_value_set_state_default));
    m.insert("pnnumonce", Box::new(control_value_set_state_default));
    m.insert("pnord", Box::new(control_value_set_state_default));
    m.insert("pnordt", Box::new(control_value_set_state_default));
    m.insert("pnprev", Box::new(control_value_set_state_default));
    m.insert("pnqc", Box::new(control_value_set_state_default));
    m.insert("pnql", Box::new(control_value_set_state_default));
    m.insert("pnqr", Box::new(control_value_set_state_default));
    m.insert("pnrestart", Box::new(control_value_set_state_default));
    m.insert("pnrnot", Box::new(control_value_set_state_default));
    m.insert("pnucltr", Box::new(control_value_set_state_default));
    m.insert("pnucrm", Box::new(control_value_set_state_default));
    m.insert("pnuld", Box::new(control_value_set_state_default));
    m.insert("pnuldash", Box::new(control_value_set_state_default));
    m.insert("pnuldashd", Box::new(control_value_set_state_default));
    m.insert("pnuldashdd", Box::new(control_value_set_state_default));
    m.insert("pnuldb", Box::new(control_value_set_state_default));
    m.insert("pnulhair", Box::new(control_value_set_state_default));
    m.insert("pnulnone", Box::new(control_value_set_state_default));
    m.insert("pnulth", Box::new(control_value_set_state_default));
    m.insert("pnulw", Box::new(control_value_set_state_default));
    m.insert("pnulwave", Box::new(control_value_set_state_default));
    m.insert("pnzodiac", Box::new(control_value_set_state_default));
    m.insert("pnzodiacd", Box::new(control_value_set_state_default));
    m.insert("pnzodiacl", Box::new(control_value_set_state_default));
    m.insert("posxc", Box::new(control_value_set_state_default));
    m.insert("posxi", Box::new(control_value_set_state_default));
    m.insert("posxl", Box::new(control_value_set_state_default));
    m.insert("posxo", Box::new(control_value_set_state_default));
    m.insert("posxr", Box::new(control_value_set_state_default));
    m.insert("posyb", Box::new(control_value_set_state_default));
    m.insert("posyc", Box::new(control_value_set_state_default));
    m.insert("posyil", Box::new(control_value_set_state_default));
    m.insert("posyin", Box::new(control_value_set_state_default));
    m.insert("posyout", Box::new(control_value_set_state_default));
    m.insert("posyt", Box::new(control_value_set_state_default));
    m.insert("prcolbl", Box::new(control_value_set_state_default));
    m.insert("printdata", Box::new(control_value_set_state_default));
    m.insert("psover", Box::new(control_value_set_state_default));
    m.insert("ptabldot", Box::new(control_value_set_state_default));
    m.insert("ptablmdot", Box::new(control_value_set_state_default));
    m.insert("ptablminus", Box::new(control_value_set_state_default));
    m.insert("ptablnone", Box::new(control_value_set_state_default));
    m.insert("ptabluscore", Box::new(control_value_set_state_default));
    m.insert("pubauto", Box::new(control_value_set_state_default));
    m.insert("pvmrg", Box::new(control_value_set_state_default));
    m.insert("pvpara", Box::new(control_value_set_state_default));
    m.insert("pvpg", Box::new(control_value_set_state_default));
    m.insert("qc", Box::new(control_value_set_state_default));
    m.insert("qd", Box::new(control_value_set_state_default));
    m.insert("qj", Box::new(control_value_set_state_default));
    m.insert("ql", Box::new(control_value_set_state_default));
    m.insert("qr", Box::new(control_value_set_state_default));
    m.insert("qt", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("rawclbgbdiag", Box::new(control_value_set_state_default));
    m.insert("rawclbgcross", Box::new(control_value_set_state_default));
    m.insert("rawclbgdcross", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkcross", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkdcross", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkhor", Box::new(control_value_set_state_default));
    m.insert("rawclbgdkvert", Box::new(control_value_set_state_default));
    m.insert("rawclbgfdiag", Box::new(control_value_set_state_default));
    m.insert("rawclbghoriz", Box::new(control_value_set_state_default));
    m.insert("rawclbgvert", Box::new(control_value_set_state_default));
    m.insert("readonlyrecommended", Box::new(control_value_set_state_default));
    m.insert("readprot", Box::new(control_value_set_state_default));
    m.insert("remdttm", Box::new(control_value_set_state_default));
    m.insert("rempersonalinfo", Box::new(control_value_set_state_default));
    m.insert("revisions", Box::new(control_value_set_state_default));
    m.insert("revprot", Box::new(control_value_set_state_default));
    m.insert("rsltbmp", Box::new(control_value_set_state_default));
    m.insert("rslthtml", Box::new(control_value_set_state_default));
    m.insert("rsltmerge", Box::new(control_value_set_state_default));
    m.insert("rsltpict", Box::new(control_value_set_state_default));
    m.insert("rsltrtf", Box::new(control_value_set_state_default));
    m.insert("rslttxt", Box::new(control_value_set_state_default));
    m.insert("rtlch", Box::new(control_value_set_state_default));
    m.insert("rtldoc", Box::new(control_value_set_state_default));
    m.insert("rtlgutter", Box::new(control_value_set_state_default));
    m.insert("rtlpar", Box::new(control_value_set_state_default));
    m.insert("rtlrow", Box::new(control_value_set_state_default));
    m.insert("rtlsect", Box::new(control_value_set_state_default));
    m.insert("saftnnalc", Box::new(control_value_set_state_default));
    m.insert("saftnnar", Box::new(control_value_set_state_default));
    m.insert("saftnnauc", Box::new(control_value_set_state_default));
    m.insert("saftnnchi", Box::new(control_value_set_state_default));
    m.insert("saftnnchosung", Box::new(control_value_set_state_default));
    m.insert("saftnncnum", Box::new(control_value_set_state_default));
    m.insert("saftnndbar", Box::new(control_value_set_state_default));
    m.insert("saftnndbnum", Box::new(control_value_set_state_default));
    m.insert("saftnndbnumd", Box::new(control_value_set_state_default));
    m.insert("saftnndbnumk", Box::new(control_value_set_state_default));
    m.insert("saftnndbnumt", Box::new(control_value_set_state_default));
    m.insert("saftnnganada", Box::new(control_value_set_state_default));
    m.insert("saftnngbnum", Box::new(control_value_set_state_default));
    m.insert("saftnngbnumd", Box::new(control_value_set_state_default));
    m.insert("saftnngbnumk", Box::new(control_value_set_state_default));
    m.insert("saftnngbnuml", Box::new(control_value_set_state_default));
    m.insert("saftnnrlc", Box::new(control_value_set_state_default));
    m.insert("saftnnruc", Box::new(control_value_set_state_default));
    m.insert("saftnnzodiac", Box::new(control_value_set_state_default));
    m.insert("saftnnzodiacd", Box::new(control_value_set_state_default));
    m.insert("saftnnzodiacl", Box::new(control_value_set_state_default));
    m.insert("saftnrestart", Box::new(control_value_set_state_default));
    m.insert("saftnrstcont", Box::new(control_value_set_state_default));
    m.insert("sautoupd", Box::new(control_value_set_state_default));
    m.insert("saveinvalidxml", Box::new(control_value_set_state_default));
    m.insert("saveprevpict", Box::new(control_value_set_state_default));
    m.insert("sbkcol", Box::new(control_value_set_state_default));
    m.insert("sbkeven", Box::new(control_value_set_state_default));
    m.insert("sbknone", Box::new(control_value_set_state_default));
    m.insert("sbkodd", Box::new(control_value_set_state_default));
    m.insert("sbkpage", Box::new(control_value_set_state_default));
    m.insert("sbys", Box::new(control_value_set_state_default));
    m.insert("scompose", Box::new(control_value_set_state_default));
    m.insert("sectd", Box::new(control_value_set_state_default));
    m.insert("sectdefaultcl", Box::new(control_value_set_state_default));
    m.insert("sectspecifycl", Box::new(control_value_set_state_default));
    // The trailing N really is part of this keyword - it is *not* a value
    m.insert("sectspecifygenN", Box::new(control_value_set_state_default));
    m.insert("sectspecifyl", Box::new(control_value_set_state_default));
    m.insert("sectunlocked", Box::new(control_value_set_state_default));
    m.insert("sftnbj", Box::new(control_value_set_state_default));
    m.insert("sftnnalc", Box::new(control_value_set_state_default));
    m.insert("sftnnar", Box::new(control_value_set_state_default));
    m.insert("sftnnauc", Box::new(control_value_set_state_default));
    m.insert("sftnnchi", Box::new(control_value_set_state_default));
    m.insert("sftnnchosung", Box::new(control_value_set_state_default));
    m.insert("sftnncnum", Box::new(control_value_set_state_default));
    m.insert("sftnndbar", Box::new(control_value_set_state_default));
    m.insert("sftnndbnum", Box::new(control_value_set_state_default));
    m.insert("sftnndbnumd", Box::new(control_value_set_state_default));
    m.insert("sftnndbnumk", Box::new(control_value_set_state_default));
    m.insert("sftnndbnumt", Box::new(control_value_set_state_default));
    m.insert("sftnnganada", Box::new(control_value_set_state_default));
    m.insert("sftnngbnum", Box::new(control_value_set_state_default));
    m.insert("sftnngbnumd", Box::new(control_value_set_state_default));
    m.insert("sftnngbnumk", Box::new(control_value_set_state_default));
    m.insert("sftnngbnuml", Box::new(control_value_set_state_default));
    m.insert("sftnnrlc", Box::new(control_value_set_state_default));
    m.insert("sftnnruc", Box::new(control_value_set_state_default));
    m.insert("sftnnzodiac", Box::new(control_value_set_state_default));
    m.insert("sftnnzodiacd", Box::new(control_value_set_state_default));
    m.insert("sftnnzodiacl", Box::new(control_value_set_state_default));
    m.insert("sftnrestart", Box::new(control_value_set_state_default));
    m.insert("sftnrstcont", Box::new(control_value_set_state_default));
    m.insert("sftnrstpg", Box::new(control_value_set_state_default));
    m.insert("sftntj", Box::new(control_value_set_state_default));
    m.insert("shidden", Box::new(control_value_set_state_default));
    m.insert("shift", Box::new(control_value_set_state_default));
    m.insert("shpbxcolumn", Box::new(control_value_set_state_default));
    m.insert("shpbxignore", Box::new(control_value_set_state_default));
    m.insert("shpbxmargin", Box::new(control_value_set_state_default));
    m.insert("shpbxpage", Box::new(control_value_set_state_default));
    m.insert("shpbyignore", Box::new(control_value_set_state_default));
    m.insert("shpbymargin", Box::new(control_value_set_state_default));
    m.insert("shpbypage", Box::new(control_value_set_state_default));
    m.insert("shpbypara", Box::new(control_value_set_state_default));
    m.insert("shplockanchor", Box::new(control_value_set_state_default));
    m.insert("slocked", Box::new(control_value_set_state_default));
    m.insert("snaptogridincell", Box::new(control_value_set_state_default));
    m.insert("softcol", Box::new(control_value_set_state_default));
    m.insert("softline", Box::new(control_value_set_state_default));
    m.insert("softpage", Box::new(control_value_set_state_default));
    m.insert("spersonal", Box::new(control_value_set_state_default));
    m.insert("spltpgpar", Box::new(control_value_set_state_default));
    m.insert("splytwnine", Box::new(control_value_set_state_default));
    m.insert("sprsbsp", Box::new(control_value_set_state_default));
    m.insert("sprslnsp", Box::new(control_value_set_state_default));
    m.insert("sprsspbf", Box::new(control_value_set_state_default));
    m.insert("sprstsm", Box::new(control_value_set_state_default));
    m.insert("sprstsp", Box::new(control_value_set_state_default));
    m.insert("spv", Box::new(control_value_set_state_default));
    m.insert("sqformat", Box::new(control_value_set_state_default));
    m.insert("sreply", Box::new(control_value_set_state_default));
    m.insert("stylelock", Box::new(control_value_set_state_default));
    m.insert("stylelockbackcomp", Box::new(control_value_set_state_default));
    m.insert("stylelockenforced", Box::new(control_value_set_state_default));
    m.insert("stylelockqfset", Box::new(control_value_set_state_default));
    m.insert("stylelocktheme", Box::new(control_value_set_state_default));
    m.insert("sub", Box::new(control_value_set_state_default));
    m.insert("subfontbysize", Box::new(control_value_set_state_default));
    m.insert("super", Box::new(control_value_set_state_default));
    m.insert("swpbdr", Box::new(control_value_set_state_default));
    m.insert("tabsnoovrlp", Box::new(control_value_set_state_default));
    m.insert("taprtl", Box::new(control_value_set_state_default));
    m.insert("tbllkbestfit", Box::new(control_value_set_state_default));
    m.insert("tbllkborder", Box::new(control_value_set_state_default));
    m.insert("tbllkcolor", Box::new(control_value_set_state_default));
    m.insert("tbllkfont", Box::new(control_value_set_state_default));
    m.insert("tbllkhdrcols", Box::new(control_value_set_state_default));
    m.insert("tbllkhdrrows", Box::new(control_value_set_state_default));
    m.insert("tbllklastcol", Box::new(control_value_set_state_default));
    m.insert("tbllklastrow", Box::new(control_value_set_state_default));
    m.insert("tbllknocolband", Box::new(control_value_set_state_default));
    m.insert("tbllknorowband", Box::new(control_value_set_state_default));
    m.insert("tbllkshading", Box::new(control_value_set_state_default));
    m.insert("tcelld", Box::new(control_value_set_state_default));
    m.insert("tcn", Box::new(control_value_set_state_default));
    m.insert("time", Box::new(control_value_set_state_default));
    m.insert("titlepg", Box::new(control_value_set_state_default));
    m.insert("tldot", Box::new(control_value_set_state_default));
    m.insert("tleq", Box::new(control_value_set_state_default));
    m.insert("tlhyph", Box::new(control_value_set_state_default));
    m.insert("tlmdot", Box::new(control_value_set_state_default));
    m.insert("tlth", Box::new(control_value_set_state_default));
    m.insert("tlul", Box::new(control_value_set_state_default));
    m.insert("toplinepunct", Box::new(control_value_set_state_default));
    m.insert("tphcol", Box::new(control_value_set_state_default));
    m.insert("tphmrg", Box::new(control_value_set_state_default));
    m.insert("tphpg", Box::new(control_value_set_state_default));
    m.insert("tposxc", Box::new(control_value_set_state_default));
    m.insert("tposxi", Box::new(control_value_set_state_default));
    m.insert("tposxl", Box::new(control_value_set_state_default));
    m.insert("tposxo", Box::new(control_value_set_state_default));
    m.insert("tposxr", Box::new(control_value_set_state_default));
    m.insert("tposyb", Box::new(control_value_set_state_default));
    m.insert("tposyc", Box::new(control_value_set_state_default));
    m.insert("tposyil", Box::new(control_value_set_state_default));
    m.insert("tposyin", Box::new(control_value_set_state_default));
    m.insert("tposyout", Box::new(control_value_set_state_default));
    m.insert("tposyt", Box::new(control_value_set_state_default));
    m.insert("tpvmrg", Box::new(control_value_set_state_default));
    m.insert("tpvpara", Box::new(control_value_set_state_default));
    m.insert("tpvpg", Box::new(control_value_set_state_default));
    m.insert("tqc", Box::new(control_value_set_state_default));
    m.insert("tqdec", Box::new(control_value_set_state_default));
    m.insert("tqr", Box::new(control_value_set_state_default));
    m.insert("transmf", Box::new(control_value_set_state_default));
    m.insert("trbgbdiag", Box::new(control_value_set_state_default));
    m.insert("trbgcross", Box::new(control_value_set_state_default));
    m.insert("trbgdcross", Box::new(control_value_set_state_default));
    m.insert("trbgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("trbgdkcross", Box::new(control_value_set_state_default));
    m.insert("trbgdkdcross", Box::new(control_value_set_state_default));
    m.insert("trbgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("trbgdkhor", Box::new(control_value_set_state_default));
    m.insert("trbgdkvert", Box::new(control_value_set_state_default));
    m.insert("trbgfdiag", Box::new(control_value_set_state_default));
    m.insert("trbghoriz", Box::new(control_value_set_state_default));
    m.insert("trbgvert", Box::new(control_value_set_state_default));
    m.insert("trbrdrb", Box::new(control_value_set_state_default));
    m.insert("trbrdrh", Box::new(control_value_set_state_default));
    m.insert("trbrdrl", Box::new(control_value_set_state_default));
    m.insert("trbrdrr", Box::new(control_value_set_state_default));
    m.insert("trbrdrt", Box::new(control_value_set_state_default));
    m.insert("trbrdrv", Box::new(control_value_set_state_default));
    m.insert("trhdr", Box::new(control_value_set_state_default));
    m.insert("trkeep", Box::new(control_value_set_state_default));
    m.insert("trkeepfollow", Box::new(control_value_set_state_default));
    m.insert("trowd", Box::new(control_value_set_state_default));
    m.insert("trqc", Box::new(control_value_set_state_default));
    m.insert("trql", Box::new(control_value_set_state_default));
    m.insert("trqr", Box::new(control_value_set_state_default));
    m.insert("truncatefontheight", Box::new(control_value_set_state_default));
    m.insert("truncex", Box::new(control_value_set_state_default));
    m.insert("tsbgbdiag", Box::new(control_value_set_state_default));
    m.insert("tsbgcross", Box::new(control_value_set_state_default));
    m.insert("tsbgdcross", Box::new(control_value_set_state_default));
    m.insert("tsbgdkbdiag", Box::new(control_value_set_state_default));
    m.insert("tsbgdkcross", Box::new(control_value_set_state_default));
    m.insert("tsbgdkdcross", Box::new(control_value_set_state_default));
    m.insert("tsbgdkfdiag", Box::new(control_value_set_state_default));
    m.insert("tsbgdkhor", Box::new(control_value_set_state_default));
    m.insert("tsbgdkvert", Box::new(control_value_set_state_default));
    m.insert("tsbgfdiag", Box::new(control_value_set_state_default));
    m.insert("tsbghoriz", Box::new(control_value_set_state_default));
    m.insert("tsbgvert", Box::new(control_value_set_state_default));
    m.insert("tsbrdrb", Box::new(control_value_set_state_default));
    m.insert("tsbrdrdgl", Box::new(control_value_set_state_default));
    m.insert("tsbrdrdgr", Box::new(control_value_set_state_default));
    m.insert("tsbrdrh", Box::new(control_value_set_state_default));
    m.insert("tsbrdrl", Box::new(control_value_set_state_default));
    m.insert("tsbrdrr", Box::new(control_value_set_state_default));
    m.insert("tsbrdrr", Box::new(control_value_set_state_default));
    m.insert("tsbrdrt", Box::new(control_value_set_state_default));
    m.insert("tsbrdrv", Box::new(control_value_set_state_default));
    m.insert("tscbandhorzeven", Box::new(control_value_set_state_default));
    m.insert("tscbandhorzodd", Box::new(control_value_set_state_default));
    m.insert("tscbandverteven", Box::new(control_value_set_state_default));
    m.insert("tscbandvertodd", Box::new(control_value_set_state_default));
    m.insert("tscfirstcol", Box::new(control_value_set_state_default));
    m.insert("tscfirstrow", Box::new(control_value_set_state_default));
    m.insert("tsclastcol", Box::new(control_value_set_state_default));
    m.insert("tsclastrow", Box::new(control_value_set_state_default));
    m.insert("tscnecell", Box::new(control_value_set_state_default));
    m.insert("tscnwcell", Box::new(control_value_set_state_default));
    m.insert("tscsecell", Box::new(control_value_set_state_default));
    m.insert("tscswcell", Box::new(control_value_set_state_default));
    m.insert("tsd", Box::new(control_value_set_state_default));
    m.insert("tsnowrap", Box::new(control_value_set_state_default));
    m.insert("tsrowd", Box::new(control_value_set_state_default));
    m.insert("tsvertalb", Box::new(control_value_set_state_default));
    m.insert("tsvertalc", Box::new(control_value_set_state_default));
    m.insert("tsvertalt", Box::new(control_value_set_state_default));
    m.insert("twoonone", Box::new(control_value_set_state_default));
    m.insert("txbxtwalways", Box::new(control_value_set_state_default));
    m.insert("txbxtwfirst", Box::new(control_value_set_state_default));
    m.insert("txbxtwfirstlast", Box::new(control_value_set_state_default));
    m.insert("txbxtwlast", Box::new(control_value_set_state_default));
    m.insert("txbxtwno", Box::new(control_value_set_state_default));
    m.insert("uld", Box::new(control_value_set_state_default));
    m.insert("ulnone", Box::new(control_value_set_state_default));
    m.insert("ulw", Box::new(control_value_set_state_default));
    m.insert("useltbaln", Box::new(control_value_set_state_default));
    m.insert("usenormstyforlist", Box::new(control_value_set_state_default));
    m.insert("usexform", Box::new(control_value_set_state_default));
    m.insert("utinl", Box::new(control_value_set_state_default));
    m.insert("vertal", Box::new(control_value_set_state_default));
    m.insert("vertalb", Box::new(control_value_set_state_default));
    m.insert("vertalc", Box::new(control_value_set_state_default));
    m.insert("vertalj", Box::new(control_value_set_state_default));
    m.insert("vertalt", Box::new(control_value_set_state_default));
    m.insert("vertdoc", Box::new(control_value_set_state_default));
    m.insert("vertsect", Box::new(control_value_set_state_default));
    m.insert("viewnobound", Box::new(control_value_set_state_default));
    m.insert("webhidden", Box::new(control_value_set_state_default));
    m.insert("widctlpar", Box::new(control_value_set_state_default));
    m.insert("widowctrl", Box::new(control_value_set_state_default));
    m.insert("wpeqn", Box::new(control_value_set_state_default));
    m.insert("wpjst", Box::new(control_value_set_state_default));
    m.insert("wpsp", Box::new(control_value_set_state_default));
    m.insert("wraparound", Box::new(control_value_set_state_default));
    m.insert("wrapdefault", Box::new(control_value_set_state_default));
    m.insert("wrapthrough", Box::new(control_value_set_state_default));
    m.insert("wraptight", Box::new(control_value_set_state_default));
    m.insert("wraptrsp", Box::new(control_value_set_state_default));
    m.insert("wrppunct", Box::new(control_value_set_state_default));
    m.insert("xmlattr", Box::new(control_value_set_state_default));
    m.insert("xmlsdttcell", Box::new(control_value_set_state_default));
    m.insert("xmlsdttpara", Box::new(control_value_set_state_default));
    m.insert("xmlsdttregular", Box::new(control_value_set_state_default));
    m.insert("xmlsdttrow", Box::new(control_value_set_state_default));
    m.insert("xmlsdttunknown", Box::new(control_value_set_state_default));
    m.insert("yxe", Box::new(control_value_set_state_default));
    // This appears to be an unofficial flag used by WordML
    m.insert("outdisponlyhtml", Box::new(control_value_set_state_default));
    // These are unofficial flags used by the macOS CocoaRTF export filter
    // https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/AttributedStrings/Tasks/RTFAndAttrStrings.html
    m.insert("glnam", Box::new(control_value_set_state_default));
    m.insert("pardirnatural", Box::new(control_value_set_state_default));
    m.insert("qnatural", Box::new(control_value_set_state_default));
    m
    };

    pub static ref TOGGLES: HashMap<&'static str, Box<StateHandler>> = {
    let mut m = HashMap::<_, Box<StateHandler>>::new();
    m.insert("ab", Box::new(control_value_set_state_default));
    m.insert("absnoovrlp", Box::new(control_value_set_state_default));
    m.insert("acaps", Box::new(control_value_set_state_default));
    m.insert("acccircle", Box::new(control_value_set_state_default));
    m.insert("acccomma", Box::new(control_value_set_state_default));
    m.insert("accdot", Box::new(control_value_set_state_default));
    m.insert("accnone", Box::new(control_value_set_state_default));
    m.insert("accunderdot", Box::new(control_value_set_state_default));
    m.insert("ai", Box::new(control_value_set_state_default));
    m.insert("aoutl", Box::new(control_value_set_state_default));
    m.insert("ascaps", Box::new(control_value_set_state_default));
    m.insert("ashad", Box::new(control_value_set_state_default));
    m.insert("aspalpha", Box::new(control_value_set_state_default));
    m.insert("aspnum", Box::new(control_value_set_state_default));
    m.insert("astrike", Box::new(control_value_set_state_default));
    m.insert("aul", Box::new(control_value_set_state_default));
    m.insert("auld", Box::new(control_value_set_state_default));
    m.insert("auldb", Box::new(control_value_set_state_default));
    m.insert("aulnone", Box::new(control_value_set_state_default));
    m.insert("aulw", Box::new(control_value_set_state_default));
    m.insert("b", Box::new(control_value_set_state_default));
    m.insert("caps", Box::new(control_value_set_state_default));
    m.insert("deleted", Box::new(control_value_set_state_default));
    m.insert("disabled", Box::new(control_value_set_state_default));
    m.insert("embo", Box::new(control_value_set_state_default));
    m.insert("htmlrtf", Box::new(control_value_set_state_default));
    m.insert("hyphauto", Box::new(control_value_set_state_default));
    m.insert("hyphcaps", Box::new(control_value_set_state_default));
    m.insert("hyphpar", Box::new(control_value_set_state_default));
    m.insert("i", Box::new(control_value_set_state_default));
    m.insert("impr", Box::new(control_value_set_state_default));
    m.insert("outl", Box::new(control_value_set_state_default));
    m.insert("pnb", Box::new(control_value_set_state_default));
    m.insert("pncaps", Box::new(control_value_set_state_default));
    m.insert("pni", Box::new(control_value_set_state_default));
    m.insert("pnscaps", Box::new(control_value_set_state_default));
    m.insert("pnstrike", Box::new(control_value_set_state_default));
    m.insert("pnul", Box::new(control_value_set_state_default));
    m.insert("protect", Box::new(control_value_set_state_default));
    m.insert("revised", Box::new(control_value_set_state_default));
    m.insert("saauto", Box::new(control_value_set_state_default));
    m.insert("sbauto", Box::new(control_value_set_state_default));
    m.insert("scaps", Box::new(control_value_set_state_default));
    m.insert("shad", Box::new(control_value_set_state_default));
    m.insert("strike", Box::new(control_value_set_state_default));
    m.insert("striked", Box::new(control_value_set_state_default));
    m.insert("trautofit", Box::new(control_value_set_state_default));
    m.insert("ul", Box::new(control_value_set_state_default));
    m.insert("uldash", Box::new(control_value_set_state_default));
    m.insert("uldashd", Box::new(control_value_set_state_default));
    m.insert("uldashdd", Box::new(control_value_set_state_default));
    m.insert("uldb", Box::new(control_value_set_state_default));
    m.insert("ulhair", Box::new(control_value_set_state_default));
    m.insert("ulhwave", Box::new(control_value_set_state_default));
    m.insert("ulldash", Box::new(control_value_set_state_default));
    m.insert("ulth", Box::new(control_value_set_state_default));
    m.insert("ulth", Box::new(control_value_set_state_default));
    m.insert("ulthd", Box::new(control_value_set_state_default));
    m.insert("ulthdash", Box::new(control_value_set_state_default));
    m.insert("ulthdashd", Box::new(control_value_set_state_default));
    m.insert("ulthdashdd", Box::new(control_value_set_state_default));
    m.insert("ulthldash", Box::new(control_value_set_state_default));
    m.insert("ululdbwave", Box::new(control_value_set_state_default));
    m.insert("ulwave", Box::new(control_value_set_state_default));
    m.insert("v", Box::new(control_value_set_state_default));
    // These are unofficial toggles used by OpenOffice RTF export filter
    m.insert("hyphmax", Box::new(control_value_set_state_default));
    m.insert("pgdscnxt", Box::new(control_value_set_state_default));
    m
    };

    pub static ref VALUES: HashMap<&'static str, Box<StateHandler>> = {
    let mut m = HashMap::<_, Box<StateHandler>>::new();
    m.insert("absh", Box::new(control_value_set_state_default));
    m.insert("absw", Box::new(control_value_set_state_default));
    m.insert("acf", Box::new(control_value_set_state_default));
    m.insert("adeff", Box::new(control_value_set_state_default));
    m.insert("adeflang", Box::new(control_value_set_state_default));
    m.insert("adn", Box::new(control_value_set_state_default));
    m.insert("aexpnd", Box::new(control_value_set_state_default));
    m.insert("af", Box::new(control_value_set_state_default));
    m.insert("afs", Box::new(control_value_set_state_default));
    m.insert("aftnstart", Box::new(control_value_set_state_default));
    m.insert("alang", Box::new(control_value_set_state_default));
    m.insert("animtext", Box::new(control_value_set_state_default));
    m.insert("ansicpg", Box::new(control_value_set_state_encoding));
    m.insert("aup", Box::new(control_value_set_state_default));
    m.insert("bin", Box::new(control_value_set_state_default));
    m.insert("binfsxn", Box::new(control_value_set_state_default));
    m.insert("binsxn", Box::new(control_value_set_state_default));
    m.insert("bkmkcolf", Box::new(control_value_set_state_default));
    m.insert("bkmkcoll", Box::new(control_value_set_state_default));
    m.insert("bliptag", Box::new(control_value_set_state_default));
    m.insert("blipupi", Box::new(control_value_set_state_default));
    m.insert("blue", Box::new(control_value_set_state_default));
    m.insert("bookfoldsheets", Box::new(control_value_set_state_default));
    m.insert("brdrart", Box::new(control_value_set_state_default));
    m.insert("brdrcf", Box::new(control_value_set_state_default));
    m.insert("brdrw", Box::new(control_value_set_state_default));
    m.insert("brsp", Box::new(control_value_set_state_default));
    m.insert("cb", Box::new(control_value_set_state_default));
    m.insert("cbpat", Box::new(control_value_set_state_default));
    m.insert("cchs", Box::new(control_value_set_state_default));
    m.insert("cellx", Box::new(control_value_set_state_default));
    m.insert("cf", Box::new(control_value_set_state_default));
    m.insert("cfpat", Box::new(control_value_set_state_default));
    m.insert("cgrid", Box::new(control_value_set_state_default));
    m.insert("charrsid", Box::new(control_value_set_state_default));
    m.insert("charscalex", Box::new(control_value_set_state_default));
    m.insert("chcbpat", Box::new(control_value_set_state_default));
    m.insert("chcfpat", Box::new(control_value_set_state_default));
    m.insert("chhres", Box::new(control_value_set_state_default));
    m.insert("chshdng", Box::new(control_value_set_state_default));
    m.insert("clcbpat", Box::new(control_value_set_state_default));
    m.insert("clcbpatraw", Box::new(control_value_set_state_default));
    m.insert("clcfpat", Box::new(control_value_set_state_default));
    m.insert("clcfpatraw", Box::new(control_value_set_state_default));
    m.insert("cldelauth", Box::new(control_value_set_state_default));
    m.insert("cldeldttm", Box::new(control_value_set_state_default));
    m.insert("clftsWidth", Box::new(control_value_set_state_default));
    m.insert("clinsauth", Box::new(control_value_set_state_default));
    m.insert("clinsdttm", Box::new(control_value_set_state_default));
    m.insert("clmrgdauth", Box::new(control_value_set_state_default));
    m.insert("clmrgddttm", Box::new(control_value_set_state_default));
    m.insert("clpadb", Box::new(control_value_set_state_default));
    m.insert("clpadfb", Box::new(control_value_set_state_default));
    m.insert("clpadfl", Box::new(control_value_set_state_default));
    m.insert("clpadfr", Box::new(control_value_set_state_default));
    m.insert("clpadft", Box::new(control_value_set_state_default));
    m.insert("clpadl", Box::new(control_value_set_state_default));
    m.insert("clpadr", Box::new(control_value_set_state_default));
    m.insert("clpadt", Box::new(control_value_set_state_default));
    m.insert("clspb", Box::new(control_value_set_state_default));
    m.insert("clspfb", Box::new(control_value_set_state_default));
    m.insert("clspfl", Box::new(control_value_set_state_default));
    m.insert("clspfr", Box::new(control_value_set_state_default));
    m.insert("clspft", Box::new(control_value_set_state_default));
    m.insert("clspl", Box::new(control_value_set_state_default));
    m.insert("clspr", Box::new(control_value_set_state_default));
    m.insert("clspt", Box::new(control_value_set_state_default));
    m.insert("clshdng", Box::new(control_value_set_state_default));
    m.insert("clshdngraw", Box::new(control_value_set_state_default));
    m.insert("clwWidth", Box::new(control_value_set_state_default));
    m.insert("colno", Box::new(control_value_set_state_default));
    m.insert("cols", Box::new(control_value_set_state_default));
    m.insert("colsr", Box::new(control_value_set_state_default));
    m.insert("colsx", Box::new(control_value_set_state_default));
    m.insert("colw", Box::new(control_value_set_state_default));
    m.insert("cpg", Box::new(control_value_set_state_default));
    m.insert("crauth", Box::new(control_value_set_state_default));
    m.insert("crdate", Box::new(control_value_set_state_default));
    m.insert("cs", Box::new(control_value_set_state_default));
    m.insert("cshade", Box::new(control_value_set_state_default));
    m.insert("ctint", Box::new(control_value_set_state_default));
    m.insert("cts", Box::new(control_value_set_state_default));
    m.insert("cufi", Box::new(control_value_set_state_default));
    m.insert("culi", Box::new(control_value_set_state_default));
    m.insert("curi", Box::new(control_value_set_state_default));
    m.insert("deff", Box::new(control_value_set_state_default));
    m.insert("deflang", Box::new(control_value_set_state_default));
    m.insert("deflangfe", Box::new(control_value_set_state_default));
    m.insert("deftab", Box::new(control_value_set_state_default));
    m.insert("delrsid", Box::new(control_value_set_state_default));
    m.insert("dfrauth", Box::new(control_value_set_state_default));
    m.insert("dfrdate", Box::new(control_value_set_state_default));
    m.insert("dfrmtxtx", Box::new(control_value_set_state_default));
    m.insert("dfrmtxty", Box::new(control_value_set_state_default));
    m.insert("dfrstart", Box::new(control_value_set_state_default));
    m.insert("dfrstop", Box::new(control_value_set_state_default));
    m.insert("dfrxst", Box::new(control_value_set_state_default));
    m.insert("dghorigin", Box::new(control_value_set_state_default));
    m.insert("dghshow", Box::new(control_value_set_state_default));
    m.insert("dghspace", Box::new(control_value_set_state_default));
    m.insert("dgvorigin", Box::new(control_value_set_state_default));
    m.insert("dgvshow", Box::new(control_value_set_state_default));
    m.insert("dgvspace", Box::new(control_value_set_state_default));
    m.insert("dibitmap", Box::new(control_value_set_state_default));
    m.insert("dn", Box::new(control_value_set_state_default));
    m.insert("doctype", Box::new(control_value_set_state_default));
    m.insert("dodhgt", Box::new(control_value_set_state_default));
    m.insert("donotembedlingdata", Box::new(control_value_set_state_default));
    m.insert("donotembedsysfont", Box::new(control_value_set_state_default));
    m.insert("dpaendl", Box::new(control_value_set_state_default));
    m.insert("dpaendw", Box::new(control_value_set_state_default));
    m.insert("dpastartl", Box::new(control_value_set_state_default));
    m.insert("dpastartw", Box::new(control_value_set_state_default));
    m.insert("dpcoa", Box::new(control_value_set_state_default));
    m.insert("dpcodescent", Box::new(control_value_set_state_default));
    m.insert("dpcolength", Box::new(control_value_set_state_default));
    m.insert("dpcooffset", Box::new(control_value_set_state_default));
    m.insert("dpcount", Box::new(control_value_set_state_default));
    m.insert("dpfillbgcb", Box::new(control_value_set_state_default));
    m.insert("dpfillbgcg", Box::new(control_value_set_state_default));
    m.insert("dpfillbgcr", Box::new(control_value_set_state_default));
    m.insert("dpfillbggray", Box::new(control_value_set_state_default));
    m.insert("dpfillfgcb", Box::new(control_value_set_state_default));
    m.insert("dpfillfgcg", Box::new(control_value_set_state_default));
    m.insert("dpfillfgcr", Box::new(control_value_set_state_default));
    m.insert("dpfillfggray", Box::new(control_value_set_state_default));
    m.insert("dpfillpat", Box::new(control_value_set_state_default));
    m.insert("dplinecob", Box::new(control_value_set_state_default));
    m.insert("dplinecog", Box::new(control_value_set_state_default));
    m.insert("dplinecor", Box::new(control_value_set_state_default));
    m.insert("dplinegray", Box::new(control_value_set_state_default));
    m.insert("dplinew", Box::new(control_value_set_state_default));
    m.insert("dppolycount", Box::new(control_value_set_state_default));
    m.insert("dpptx", Box::new(control_value_set_state_default));
    m.insert("dppty", Box::new(control_value_set_state_default));
    m.insert("dpshadx", Box::new(control_value_set_state_default));
    m.insert("dpshady", Box::new(control_value_set_state_default));
    m.insert("dptxbxmar", Box::new(control_value_set_state_default));
    m.insert("dpx", Box::new(control_value_set_state_default));
    m.insert("dpxsize", Box::new(control_value_set_state_default));
    m.insert("dpy", Box::new(control_value_set_state_default));
    m.insert("dpysize", Box::new(control_value_set_state_default));
    m.insert("dropcapli", Box::new(control_value_set_state_default));
    m.insert("dropcapt", Box::new(control_value_set_state_default));
    m.insert("ds", Box::new(control_value_set_state_default));
    m.insert("dxfrtext", Box::new(control_value_set_state_default));
    m.insert("dy", Box::new(control_value_set_state_default));
    m.insert("edmins", Box::new(control_value_set_state_default));
    m.insert("enforceprot", Box::new(control_value_set_state_default));
    m.insert("expnd", Box::new(control_value_set_state_default));
    m.insert("expndtw", Box::new(control_value_set_state_default));
    m.insert("f", Box::new(control_value_set_state_default));
    m.insert("fbias", Box::new(control_value_set_state_default));
    m.insert("fcharset", Box::new(control_value_set_state_default));
    m.insert("fcs", Box::new(control_value_set_state_default));
    m.insert("fet", Box::new(control_value_set_state_default));
    m.insert("ffdefres", Box::new(control_value_set_state_default));
    m.insert("ffhaslistbox", Box::new(control_value_set_state_default));
    m.insert("ffhps", Box::new(control_value_set_state_default));
    m.insert("ffmaxlen", Box::new(control_value_set_state_default));
    m.insert("ffownhelp", Box::new(control_value_set_state_default));
    m.insert("ffownstat", Box::new(control_value_set_state_default));
    m.insert("ffprot", Box::new(control_value_set_state_default));
    m.insert("ffrecalc", Box::new(control_value_set_state_default));
    m.insert("ffres", Box::new(control_value_set_state_default));
    m.insert("ffsize", Box::new(control_value_set_state_default));
    m.insert("fftype", Box::new(control_value_set_state_default));
    m.insert("fftypetxt", Box::new(control_value_set_state_default));
    m.insert("fi", Box::new(control_value_set_state_default));
    m.insert("fid", Box::new(control_value_set_state_default));
    m.insert("fittext", Box::new(control_value_set_state_default));
    m.insert("fn", Box::new(control_value_set_state_default));
    m.insert("footery", Box::new(control_value_set_state_default));
    m.insert("fosnum", Box::new(control_value_set_state_default));
    m.insert("fprq", Box::new(control_value_set_state_default));
    m.insert("frelative", Box::new(control_value_set_state_default));
    m.insert("fromhtml", Box::new(control_value_set_state_default));
    m.insert("fs", Box::new(control_value_set_state_default));
    m.insert("ftnstart", Box::new(control_value_set_state_default));
    m.insert("gcw", Box::new(control_value_set_state_default));
    m.insert("green", Box::new(control_value_set_state_default));
    m.insert("grfdocevents", Box::new(control_value_set_state_default));
    m.insert("gutter", Box::new(control_value_set_state_default));
    m.insert("guttersxn", Box::new(control_value_set_state_default));
    m.insert("headery", Box::new(control_value_set_state_default));
    m.insert("highlight", Box::new(control_value_set_state_default));
    m.insert("horzvert", Box::new(control_value_set_state_default));
    m.insert("hr", Box::new(control_value_set_state_default));
    m.insert("hres", Box::new(control_value_set_state_default));
    m.insert("hyphconsec", Box::new(control_value_set_state_default));
    m.insert("hyphhotz", Box::new(control_value_set_state_default));
    m.insert("id", Box::new(control_value_set_state_default));
    m.insert("ignoremixedcontent", Box::new(control_value_set_state_default));
    m.insert("ilfomacatclnup", Box::new(control_value_set_state_default));
    m.insert("ilvl", Box::new(control_value_set_state_default));
    m.insert("insrsid", Box::new(control_value_set_state_default));
    m.insert("ipgp", Box::new(control_value_set_state_default));
    m.insert("irowband", Box::new(control_value_set_state_default));
    m.insert("irow", Box::new(control_value_set_state_default));
    m.insert("itap", Box::new(control_value_set_state_default));
    m.insert("kerning", Box::new(control_value_set_state_default));
    m.insert("ksulang", Box::new(control_value_set_state_default));
    m.insert("lang", Box::new(control_value_set_state_default));
    m.insert("langfe", Box::new(control_value_set_state_default));
    m.insert("langfenp", Box::new(control_value_set_state_default));
    m.insert("langnp", Box::new(control_value_set_state_default));
    m.insert("lbr", Box::new(control_value_set_state_default));
    m.insert("level", Box::new(control_value_set_state_default));
    m.insert("levelfollow", Box::new(control_value_set_state_default));
    m.insert("levelindent", Box::new(control_value_set_state_default));
    m.insert("leveljc", Box::new(control_value_set_state_default));
    m.insert("leveljcn", Box::new(control_value_set_state_default));
    m.insert("levellegal", Box::new(control_value_set_state_default));
    m.insert("levelnfc", Box::new(control_value_set_state_default));
    m.insert("levelnfcn", Box::new(control_value_set_state_default));
    m.insert("levelnorestart", Box::new(control_value_set_state_default));
    m.insert("levelold", Box::new(control_value_set_state_default));
    m.insert("levelpicture", Box::new(control_value_set_state_default));
    m.insert("levelprev", Box::new(control_value_set_state_default));
    m.insert("levelprevspace", Box::new(control_value_set_state_default));
    m.insert("levelspace", Box::new(control_value_set_state_default));
    m.insert("levelstartat", Box::new(control_value_set_state_default));
    m.insert("leveltemplateid", Box::new(control_value_set_state_default));
    m.insert("li", Box::new(control_value_set_state_default));
    m.insert("linemod", Box::new(control_value_set_state_default));
    m.insert("linestart", Box::new(control_value_set_state_default));
    m.insert("linestarts", Box::new(control_value_set_state_default));
    m.insert("linex", Box::new(control_value_set_state_default));
    m.insert("lin", Box::new(control_value_set_state_default));
    m.insert("lisa", Box::new(control_value_set_state_default));
    m.insert("lisb", Box::new(control_value_set_state_default));
    m.insert("listid", Box::new(control_value_set_state_default));
    m.insert("listoverridecount", Box::new(control_value_set_state_default));
    m.insert("listoverrideformat", Box::new(control_value_set_state_default));
    m.insert("listrestarthdn", Box::new(control_value_set_state_default));
    m.insert("listsimple", Box::new(control_value_set_state_default));
    m.insert("liststyleid", Box::new(control_value_set_state_default));
    m.insert("listtemplateid", Box::new(control_value_set_state_default));
    m.insert("ls", Box::new(control_value_set_state_default));
    m.insert("lsdlocked", Box::new(control_value_set_state_default));
    m.insert("lsdlockeddef", Box::new(control_value_set_state_default));
    m.insert("lsdpriority", Box::new(control_value_set_state_default));
    m.insert("lsdprioritydef", Box::new(control_value_set_state_default));
    m.insert("lsdqformat", Box::new(control_value_set_state_default));
    m.insert("lsdqformatdef", Box::new(control_value_set_state_default));
    m.insert("lsdsemihidden", Box::new(control_value_set_state_default));
    m.insert("lsdsemihiddendef", Box::new(control_value_set_state_default));
    m.insert("lsdstimax", Box::new(control_value_set_state_default));
    m.insert("lsdunhideused", Box::new(control_value_set_state_default));
    m.insert("lsdunhideuseddef", Box::new(control_value_set_state_default));
    m.insert("margb", Box::new(control_value_set_state_default));
    m.insert("margbsxn", Box::new(control_value_set_state_default));
    m.insert("margl", Box::new(control_value_set_state_default));
    m.insert("marglsxn", Box::new(control_value_set_state_default));
    m.insert("margr", Box::new(control_value_set_state_default));
    m.insert("margrsxn", Box::new(control_value_set_state_default));
    m.insert("margSz", Box::new(control_value_set_state_default));
    m.insert("margt", Box::new(control_value_set_state_default));
    m.insert("margtsxn", Box::new(control_value_set_state_default));
    m.insert("mbrk", Box::new(control_value_set_state_default));
    m.insert("mbrkBin", Box::new(control_value_set_state_default));
    m.insert("mbrkBinSub", Box::new(control_value_set_state_default));
    m.insert("mcGp", Box::new(control_value_set_state_default));
    m.insert("mcGpRule", Box::new(control_value_set_state_default));
    m.insert("mcSp", Box::new(control_value_set_state_default));
    m.insert("mdefJc", Box::new(control_value_set_state_default));
    m.insert("mdiffSty", Box::new(control_value_set_state_default));
    // Microsoft's Tom Jebo confirmed that mdispdef in the spec document is a typo and it
    // should be mdispDef, but that they would not be fixing it
    // So we'll support both
    // https://qa.social.msdn.microsoft.com/Forums/en-US/7772c72e-45b2-4ee2-aa4d-3fe8e5753811/rtf-191-mdispdef-control-word?forum=os_specifications
    m.insert("mdispdef", Box::new(control_value_set_state_default));
    m.insert("mdispDef", Box::new(control_value_set_state_default));
    m.insert("min", Box::new(control_value_set_state_default));
    m.insert("minterSp", Box::new(control_value_set_state_default));
    m.insert("mintLim", Box::new(control_value_set_state_default));
    m.insert("mintraSp", Box::new(control_value_set_state_default));
    m.insert("mjc", Box::new(control_value_set_state_default));
    m.insert("mlMargin", Box::new(control_value_set_state_default));
    m.insert("mmathFont", Box::new(control_value_set_state_default));
    m.insert("mmerrors", Box::new(control_value_set_state_default));
    m.insert("mmjdsotype", Box::new(control_value_set_state_default));
    m.insert("mmodsoactive", Box::new(control_value_set_state_default));
    m.insert("mmodsocoldelim", Box::new(control_value_set_state_default));
    m.insert("mmodsocolumn", Box::new(control_value_set_state_default));
    m.insert("mmodsodynaddr", Box::new(control_value_set_state_default));
    m.insert("mmodsofhdr", Box::new(control_value_set_state_default));
    m.insert("mmodsofmcolumn", Box::new(control_value_set_state_default));
    m.insert("mmodsohash", Box::new(control_value_set_state_default));
    m.insert("mmodsolid", Box::new(control_value_set_state_default));
    m.insert("mmreccur", Box::new(control_value_set_state_default));
    m.insert("mnaryLim", Box::new(control_value_set_state_default));
    m.insert("mo", Box::new(control_value_set_state_default));
    m.insert("mpostSp", Box::new(control_value_set_state_default));
    m.insert("mpreSp", Box::new(control_value_set_state_default));
    m.insert("mrMargin", Box::new(control_value_set_state_default));
    m.insert("mrSp", Box::new(control_value_set_state_default));
    m.insert("mrSpRule", Box::new(control_value_set_state_default));
    m.insert("mscr", Box::new(control_value_set_state_default));
    m.insert("msmallFrac", Box::new(control_value_set_state_default));
    m.insert("msty", Box::new(control_value_set_state_default));
    m.insert("mvauth", Box::new(control_value_set_state_default));
    m.insert("mvdate", Box::new(control_value_set_state_default));
    m.insert("mwrapIndent", Box::new(control_value_set_state_default));
    m.insert("mwrapRight", Box::new(control_value_set_state_default));
    m.insert("nofchars", Box::new(control_value_set_state_default));
    m.insert("nofcharsws", Box::new(control_value_set_state_default));
    m.insert("nofpages", Box::new(control_value_set_state_default));
    m.insert("nofwords", Box::new(control_value_set_state_default));
    m.insert("objalign", Box::new(control_value_set_state_default));
    m.insert("objcropb", Box::new(control_value_set_state_default));
    m.insert("objcropl", Box::new(control_value_set_state_default));
    m.insert("objcropr", Box::new(control_value_set_state_default));
    m.insert("objcropt", Box::new(control_value_set_state_default));
    m.insert("objh", Box::new(control_value_set_state_default));
    m.insert("objscalex", Box::new(control_value_set_state_default));
    m.insert("objscaley", Box::new(control_value_set_state_default));
    m.insert("objtransy", Box::new(control_value_set_state_default));
    m.insert("objw", Box::new(control_value_set_state_default));
    m.insert("ogutter", Box::new(control_value_set_state_default));
    m.insert("outlinelevel", Box::new(control_value_set_state_default));
    m.insert("paperh", Box::new(control_value_set_state_default));
    m.insert("paperw", Box::new(control_value_set_state_default));
    m.insert("pararsid", Box::new(control_value_set_state_default));
    m.insert("pgbrdropt", Box::new(control_value_set_state_default));
    m.insert("pghsxn", Box::new(control_value_set_state_default));
    m.insert("pgnhn", Box::new(control_value_set_state_default));
    m.insert("pgnstart", Box::new(control_value_set_state_default));
    m.insert("pgnstarts", Box::new(control_value_set_state_default));
    m.insert("pgnx", Box::new(control_value_set_state_default));
    m.insert("pgny", Box::new(control_value_set_state_default));
    m.insert("pgwsxn", Box::new(control_value_set_state_default));
    m.insert("picbpp", Box::new(control_value_set_state_default));
    m.insert("piccropb", Box::new(control_value_set_state_default));
    m.insert("piccropl", Box::new(control_value_set_state_default));
    m.insert("piccropr", Box::new(control_value_set_state_default));
    m.insert("piccropt", Box::new(control_value_set_state_default));
    m.insert("pich", Box::new(control_value_set_state_default));
    m.insert("pichgoal", Box::new(control_value_set_state_default));
    m.insert("picscalex", Box::new(control_value_set_state_default));
    m.insert("picscaley", Box::new(control_value_set_state_default));
    m.insert("picw", Box::new(control_value_set_state_default));
    m.insert("picwgoal", Box::new(control_value_set_state_default));
    m.insert("pmmetafile", Box::new(control_value_set_state_default));
    m.insert("pncf", Box::new(control_value_set_state_default));
    m.insert("pnf", Box::new(control_value_set_state_default));
    m.insert("pnfs", Box::new(control_value_set_state_default));
    m.insert("pnindent", Box::new(control_value_set_state_default));
    m.insert("pnlvl", Box::new(control_value_set_state_default));
    m.insert("pnrauth", Box::new(control_value_set_state_default));
    m.insert("pnrdate", Box::new(control_value_set_state_default));
    m.insert("pnrnfc", Box::new(control_value_set_state_default));
    m.insert("pnrpnbr", Box::new(control_value_set_state_default));
    m.insert("pnrrgb", Box::new(control_value_set_state_default));
    m.insert("pnrstart", Box::new(control_value_set_state_default));
    m.insert("pnrstop", Box::new(control_value_set_state_default));
    m.insert("pnrxst", Box::new(control_value_set_state_default));
    m.insert("pnsp", Box::new(control_value_set_state_default));
    m.insert("pnstart", Box::new(control_value_set_state_default));
    m.insert("posnegx", Box::new(control_value_set_state_default));
    m.insert("posnegy", Box::new(control_value_set_state_default));
    m.insert("posx", Box::new(control_value_set_state_default));
    m.insert("posy", Box::new(control_value_set_state_default));
    m.insert("prauth", Box::new(control_value_set_state_default));
    m.insert("prdate", Box::new(control_value_set_state_default));
    m.insert("proptype", Box::new(control_value_set_state_default));
    m.insert("protlevel", Box::new(control_value_set_state_default));
    m.insert("psz", Box::new(control_value_set_state_default));
    m.insert("pwd", Box::new(control_value_set_state_default));
    m.insert("qk", Box::new(control_value_set_state_default));
    m.insert("red", Box::new(control_value_set_state_default));
    m.insert("relyonvml", Box::new(control_value_set_state_default));
    m.insert("revauth", Box::new(control_value_set_state_default));
    m.insert("revauthdel", Box::new(control_value_set_state_default));
    m.insert("revbar", Box::new(control_value_set_state_default));
    m.insert("revdttm", Box::new(control_value_set_state_default));
    m.insert("revdttmdel", Box::new(control_value_set_state_default));
    m.insert("revprop", Box::new(control_value_set_state_default));
    m.insert("ri", Box::new(control_value_set_state_default));
    m.insert("rin", Box::new(control_value_set_state_default));
    m.insert("rsid", Box::new(control_value_set_state_default));
    m.insert("rsidroot", Box::new(control_value_set_state_default));
    m.insert("s", Box::new(control_value_set_state_default));
    m.insert("sa", Box::new(control_value_set_state_default));
    m.insert("saftnstart", Box::new(control_value_set_state_default));
    m.insert("sb", Box::new(control_value_set_state_default));
    m.insert("sbasedon", Box::new(control_value_set_state_default));
    m.insert("sec", Box::new(control_value_set_state_default));
    m.insert("sectexpand", Box::new(control_value_set_state_default));
    m.insert("sectlinegrid", Box::new(control_value_set_state_default));
    m.insert("sectrsid", Box::new(control_value_set_state_default));
    m.insert("sftnstart", Box::new(control_value_set_state_default));
    m.insert("shading", Box::new(control_value_set_state_default));
    m.insert("showplaceholdtext", Box::new(control_value_set_state_default));
    m.insert("showxmlerrors", Box::new(control_value_set_state_default));
    m.insert("shpbottom", Box::new(control_value_set_state_default));
    m.insert("shpfblwtxt", Box::new(control_value_set_state_default));
    m.insert("shpfhdr", Box::new(control_value_set_state_default));
    m.insert("shpleft", Box::new(control_value_set_state_default));
    m.insert("shplid", Box::new(control_value_set_state_default));
    m.insert("shpright", Box::new(control_value_set_state_default));
    m.insert("shptop", Box::new(control_value_set_state_default));
    m.insert("shpwrk", Box::new(control_value_set_state_default));
    m.insert("shpwr", Box::new(control_value_set_state_default));
    m.insert("shpz", Box::new(control_value_set_state_default));
    m.insert("sl", Box::new(control_value_set_state_default));
    m.insert("slink", Box::new(control_value_set_state_default));
    m.insert("slmult", Box::new(control_value_set_state_default));
    m.insert("snext", Box::new(control_value_set_state_default));
    m.insert("softlheight", Box::new(control_value_set_state_default));
    m.insert("spriority", Box::new(control_value_set_state_default));
    m.insert("srauth", Box::new(control_value_set_state_default));
    m.insert("srdate", Box::new(control_value_set_state_default));
    m.insert("ssemihidden", Box::new(control_value_set_state_default));
    m.insert("stextflow", Box::new(control_value_set_state_default));
    m.insert("stshfbi", Box::new(control_value_set_state_default));
    m.insert("stshfdbch", Box::new(control_value_set_state_default));
    m.insert("stshfhich", Box::new(control_value_set_state_default));
    m.insert("stshfloch", Box::new(control_value_set_state_default));
    m.insert("stylesortmethod", Box::new(control_value_set_state_default));
    m.insert("styrsid", Box::new(control_value_set_state_default));
    m.insert("subdocument", Box::new(control_value_set_state_default));
    m.insert("sunhideused", Box::new(control_value_set_state_default));
    m.insert("tb", Box::new(control_value_set_state_default));
    m.insert("tblind", Box::new(control_value_set_state_default));
    m.insert("tblindtype", Box::new(control_value_set_state_default));
    m.insert("tblrsid", Box::new(control_value_set_state_default));
    m.insert("tcf", Box::new(control_value_set_state_default));
    m.insert("tcl", Box::new(control_value_set_state_default));
    m.insert("tdfrmtxtBottom", Box::new(control_value_set_state_default));
    m.insert("tdfrmtxtLeft", Box::new(control_value_set_state_default));
    m.insert("tdfrmtxtRight", Box::new(control_value_set_state_default));
    m.insert("tdfrmtxtTop", Box::new(control_value_set_state_default));
    m.insert("themelang", Box::new(control_value_set_state_default));
    m.insert("themelangcs", Box::new(control_value_set_state_default));
    m.insert("themelangfe", Box::new(control_value_set_state_default));
    m.insert("tposnegx", Box::new(control_value_set_state_default));
    m.insert("tposnegy", Box::new(control_value_set_state_default));
    m.insert("tposx", Box::new(control_value_set_state_default));
    m.insert("tposy", Box::new(control_value_set_state_default));
    m.insert("trackformatting", Box::new(control_value_set_state_default));
    m.insert("trackmoves", Box::new(control_value_set_state_default));
    m.insert("trauth", Box::new(control_value_set_state_default));
    m.insert("trcbpat", Box::new(control_value_set_state_default));
    m.insert("trcfpat", Box::new(control_value_set_state_default));
    m.insert("trdate", Box::new(control_value_set_state_default));
    m.insert("trftsWidthA", Box::new(control_value_set_state_default));
    m.insert("trftsWidthB", Box::new(control_value_set_state_default));
    m.insert("trftsWidth", Box::new(control_value_set_state_default));
    m.insert("trgaph", Box::new(control_value_set_state_default));
    m.insert("trleft", Box::new(control_value_set_state_default));
    m.insert("trpaddb", Box::new(control_value_set_state_default));
    m.insert("trpaddfb", Box::new(control_value_set_state_default));
    m.insert("trpaddfl", Box::new(control_value_set_state_default));
    m.insert("trpaddfr", Box::new(control_value_set_state_default));
    m.insert("trpaddft", Box::new(control_value_set_state_default));
    m.insert("trpaddl", Box::new(control_value_set_state_default));
    m.insert("trpaddr", Box::new(control_value_set_state_default));
    m.insert("trpaddt", Box::new(control_value_set_state_default));
    m.insert("trpadob", Box::new(control_value_set_state_default));
    m.insert("trpadofb", Box::new(control_value_set_state_default));
    m.insert("trpadofl", Box::new(control_value_set_state_default));
    m.insert("trpadofr", Box::new(control_value_set_state_default));
    m.insert("trpadoft", Box::new(control_value_set_state_default));
    m.insert("trpadol", Box::new(control_value_set_state_default));
    m.insert("trpador", Box::new(control_value_set_state_default));
    m.insert("trpadot", Box::new(control_value_set_state_default));
    m.insert("trpat", Box::new(control_value_set_state_default));
    m.insert("trrh", Box::new(control_value_set_state_default));
    m.insert("trshdng", Box::new(control_value_set_state_default));
    m.insert("trspdb", Box::new(control_value_set_state_default));
    m.insert("trspdfb", Box::new(control_value_set_state_default));
    m.insert("trspdfl", Box::new(control_value_set_state_default));
    m.insert("trspdfr", Box::new(control_value_set_state_default));
    m.insert("trspdft", Box::new(control_value_set_state_default));
    m.insert("trspdl", Box::new(control_value_set_state_default));
    m.insert("trspdr", Box::new(control_value_set_state_default));
    m.insert("trspdt", Box::new(control_value_set_state_default));
    m.insert("trspob", Box::new(control_value_set_state_default));
    m.insert("trspofb", Box::new(control_value_set_state_default));
    m.insert("trspofl", Box::new(control_value_set_state_default));
    m.insert("trspofr", Box::new(control_value_set_state_default));
    m.insert("trspoft", Box::new(control_value_set_state_default));
    m.insert("trspol", Box::new(control_value_set_state_default));
    m.insert("trspor", Box::new(control_value_set_state_default));
    m.insert("trspot", Box::new(control_value_set_state_default));
    m.insert("trwWidthA", Box::new(control_value_set_state_default));
    m.insert("trwWidthB", Box::new(control_value_set_state_default));
    m.insert("trwWidth", Box::new(control_value_set_state_default));
    m.insert("ts", Box::new(control_value_set_state_default));
    m.insert("tscbandsh", Box::new(control_value_set_state_default));
    m.insert("tscbandsv", Box::new(control_value_set_state_default));
    m.insert("tscellcbpat", Box::new(control_value_set_state_default));
    m.insert("tscellcfpat", Box::new(control_value_set_state_default));
    m.insert("tscellpaddb", Box::new(control_value_set_state_default));
    m.insert("tscellpaddfb", Box::new(control_value_set_state_default));
    m.insert("tscellpaddfl", Box::new(control_value_set_state_default));
    m.insert("tscellpaddfr", Box::new(control_value_set_state_default));
    m.insert("tscellpaddft", Box::new(control_value_set_state_default));
    m.insert("tscellpaddl", Box::new(control_value_set_state_default));
    m.insert("tscellpaddr", Box::new(control_value_set_state_default));
    m.insert("tscellpaddt", Box::new(control_value_set_state_default));
    m.insert("tscellpct", Box::new(control_value_set_state_default));
    m.insert("tscellwidth", Box::new(control_value_set_state_default));
    m.insert("tscellwidthfts", Box::new(control_value_set_state_default));
    m.insert("twoinone", Box::new(control_value_set_state_default));
    m.insert("tx", Box::new(control_value_set_state_default));
    m.insert("u", Box::new(control_symbol_write_unicode_char));
    m.insert("uc", Box::new(control_value_set_state_default));
    m.insert("ulc", Box::new(control_value_set_state_default));
    m.insert("up", Box::new(control_value_set_state_default));
    m.insert("urtf", Box::new(control_value_set_state_default));
    m.insert("validatexml", Box::new(control_value_set_state_default));
    m.insert("vern", Box::new(control_value_set_state_default));
    m.insert("version", Box::new(control_value_set_state_default));
    m.insert("viewbksp", Box::new(control_value_set_state_default));
    m.insert("viewkind", Box::new(control_value_set_state_default));
    m.insert("viewscale", Box::new(control_value_set_state_default));
    m.insert("viewzk", Box::new(control_value_set_state_default));
    m.insert("wbitmap", Box::new(control_value_set_state_default));
    m.insert("wbmbitspixel", Box::new(control_value_set_state_default));
    m.insert("wbmplanes", Box::new(control_value_set_state_default));
    m.insert("wbmwidthbyte", Box::new(control_value_set_state_default));
    m.insert("wmetafile", Box::new(control_value_set_state_default));
    m.insert("xef", Box::new(control_value_set_state_default));
    m.insert("xmlattrns", Box::new(control_value_set_state_default));
    m.insert("xmlns", Box::new(control_value_set_state_default));
    m.insert("yr", Box::new(control_value_set_state_default));
    m.insert("yts", Box::new(control_value_set_state_default));
    // These are unofficial values used by the macOS CocoaRTF export filter
    // https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/AttributedStrings/Tasks/RTFAndAttrStrings.html
    m.insert("AppleTypeServicesU", Box::new(control_value_set_state_default));
    m.insert("CocoaLigature", Box::new(control_value_set_state_default));
    m.insert("cocoartf", Box::new(control_value_set_state_default));
    m.insert("cocoasubrtf", Box::new(control_value_set_state_default));
    m.insert("expansion", Box::new(control_value_set_state_default));
    m.insert("fsmilli", Box::new(control_value_set_state_default));
    m.insert("glcol", Box::new(control_value_set_state_default));
    m.insert("obliqueness", Box::new(control_value_set_state_default));
    m.insert("pardeftab", Box::new(control_value_set_state_default));
    m.insert("readonlydoc", Box::new(control_value_set_state_default));
    m.insert("shadr", Box::new(control_value_set_state_default));
    m.insert("shadx", Box::new(control_value_set_state_default));
    m.insert("shady", Box::new(control_value_set_state_default));
    m.insert("slleading", Box::new(control_value_set_state_default));
    m.insert("slmaximum", Box::new(control_value_set_state_default));
    m.insert("slminimum", Box::new(control_value_set_state_default));
    m.insert("strikec", Box::new(control_value_set_state_default));
    m.insert("strikestyle", Box::new(control_value_set_state_default));
    m.insert("strokec", Box::new(control_value_set_state_default));
    m.insert("strokewidth", Box::new(control_value_set_state_default));
    m.insert("ulstyle", Box::new(control_value_set_state_default));
    m.insert("viewh", Box::new(control_value_set_state_default));
    m.insert("vieww", Box::new(control_value_set_state_default));
    m.insert("width", Box::new(control_value_set_state_default));
    m.insert("height", Box::new(control_value_set_state_default));
    // These are unofficial values used by OpenOffice RTF export filter
    m.insert("hyphlead", Box::new(control_value_set_state_default));
    m.insert("hyphtrail", Box::new(control_value_set_state_default));
    m.insert("pgdscuse", Box::new(control_value_set_state_default));
    m
    };
}

fn handler(name: &str) -> Option<Box<StateHandler>> {
    if let Some(dest_handler) = DESTINATIONS.get(name) {
        Some(Box::new(dest_handler))
    } else if let Some(symbol_handler) = SYMBOLS.get(name) {
        Some(Box::new(symbol_handler))
    } else if let Some(value_handler) = VALUES.get(name) {
        Some(Box::new(value_handler))
    } else if let Some(flag_handler) = FLAGS.get(name) {
        Some(Box::new(flag_handler))
    } else if let Some(toggle_handler) = TOGGLES.get(name) {
        Some(Box::new(toggle_handler))
    } else {
        None
    }
}

fn control_flag_set_state_encoding(state: &mut Group, name: &str, arg: Option<i32>) {
    match name {
        "ansi" => {
            // It's possible that this is supposed to be translated to the host's
            // preferred language codepage, but I think that's only on write, and
            // is supposed to be followed up by a codepage.  I think in the absence
            // of a specific codepage, it should default to 1252 (Western European)
            state.set_codepage(1252u16)
        }
        "pc" => {
            // IBM PC codepage 437
            state.set_codepage(437u16)
        }
        "pca" => {
            // IBM PC codepage 850
            state.set_codepage(850u16)
        }
        "mac" => {
            // encoding_rs suggests that the "macintosh" encoding equates to codepage 10000
            state.set_codepage(10000u16)
        }
        _ => {
            panic!("Programmer error: {} was indicated as an encoding-related control flag, without adding an encoding mapping for it.", name)
        }
    }
    state.set_value(name, arg);
}

fn control_value_set_state_default(state: &mut Group, name: &str, arg: Option<i32>) {
    state.set_value(name, arg);
}

fn control_value_set_state_encoding(state: &mut Group, name: &str, arg: Option<i32>) {
    if let "ansicpg" = name {
        state.set_codepage(arg.unwrap_or(1252i32) as u16)
    } else {
        panic!("Programmer error: {} was indicated as an encoding-related control value, without adding an encoding mapping for it.", name)
    }

    state.set_value(name, arg);
}

fn control_word_ignore(_state: &mut Group, _name: &str, _arg: Option<i32>) {}

fn control_value_set_state_and_write_ansi_char(state: &mut Group, name: &str, arg: Option<i32>) {
    let encoding = state.encoding();
    control_symbol_write_ansi_char(state, name, arg);
    control_value_set_state_encoding(state, "ansicpg", Some(1252));
    state.set_encoding(encoding);
}

fn control_symbol_write_ansi_char(state: &mut Group, name: &str, arg: Option<i32>) {
    let arg_byte = arg.map(|n| [(n & 0xFF) as u8]).unwrap_or([0u8]);
    let opt_bytes: Option<&[u8]> = match name {
        "'" => Some(&arg_byte), // ANSI hex escape
        "\"" => Some(b"\""),    // Referenced, but not formally defined mapping in spec
        "\\" => Some(b"\\"),
        "_" => Some(b"-"), // Non-breaking hyphen
        "{" => Some(b"{"),
        "}" => Some(b"}"),
        "~" => Some(b" "),         // Non-breaking space
        "bullet" => Some(b"\x95"), // Pre-defined ANSI mapping in spec
        "emdash" => Some(b"\x97"), // Pre-defined ANSI mapping in spec
        "emspace" => Some(b"  "),
        "enspace" => Some(b" "),
        "endash" => Some(b"\x96"),    // Pre-defined ANSI mapping in spec
        "ldblquote" => Some(b"\x93"), // Pre-defined ANSI mapping in spec
        "line" => Some(b"\n"),
        "lquote" => Some(b"\x91"), // Pre-defined ANSI mapping in spec
        "page" => Some(b"\n\n"),
        "par" => Some(b"\n"),
        "rdblquote" => Some(b"\x94"), // Pre-defined ANSI mapping in spec
        "rquote" => Some(b"\x92"),    // Pre-defined ANSI mapping in spec
        "sect" => Some(b"\n\n"),
        "row" => Some(b"\n "),
        "tab" => Some(b"\t"),   // Unofficial mapping for ending a table row
        "cell" => Some(b"\t"),  // Unofficial mapping for separating table row cells
        "ls" => Some(b"\x95 "), // Unofficial mapping for list entry
        "\n" => Some(b"\n"),    // Semi-official compatibility mapping, same as \par
        "\r" => Some(b"\n"),    // Semi-official compatibility mapping, same as \par
        "\t" => Some(b"\t"),    // Semi-official compatibility mapping
        " " => Some(b" "),      // Semi-official compatibility mapping
        "/" => Some(b"/"),      // Unsupported, but used symbol mapping
        _ => {
            panic!("Unsupported ANSI char mapping requested: {}", name);
        }
    };

    if let Some(bytes) = opt_bytes {
        state.write(bytes, None);
    }
}

/// Write a unicode character (\u) to current destination
///
/// NB. does not handle \uc skipping or unicode values > 32767
fn control_symbol_write_unicode_char(state: &mut Group, _name: &str, arg: Option<i32>) {
    if let Some(codepoint) = arg {
        if let Some(c) = std::char::from_u32(codepoint as u32) {
            let mut b = [0; 4];
            let s = c.encode_utf8(&mut b);
            state.write(s.as_bytes(), Some(encoding_rs::UTF_8));
        }
    }
}

fn control_symbol_next_control_is_optional(state: &mut Group, _name: &str, _arg: Option<i32>) {
    state.set_ignore_next();
}

fn destination_control_set_state_encoding(state: &mut Group, name: &str, _arg: Option<i32>) {
    state.set_destination(name, true);
}

fn destination_control_set_state_default(state: &mut Group, name: &str, _arg: Option<i32>) {
    state.set_destination(name, false);
}

fn destination_control_and_value_set_state_default(
    state: &mut Group,
    name: &str,
    arg: Option<i32>,
) {
    state.set_destination(name, false);
    state.set_value(name, arg);
}

#[cfg(test)]
pub mod test {

    use super::*;

    #[test]
    pub fn test_comment() {
        let source = r#"{\rtf1\ansi\ansicpg1252\cocoartf2578
\cocoatextscaling0\cocoaplatform0{\fonttbl\f0\froman\fcharset0 Palatino-Roman;\f1\froman\fcharset0 Palatino-Italic;}
{\colortbl;\red255\green255\blue255;}
{\*\expandedcolortbl;;}
\pard\tx360\tx720\tx1080\tx1440\tx1800\tx2160\tx2880\tx3600\tx4320\fi360\sl264\slmult1\pardirnatural\partightenfactor0

\f0\fs26 \cf0 This is commented-on {\field{\*\fldinst{HYPERLINK "scrivcmt://3320CF04-2AE2-4D08-A1A4-3A5CFB9F43A6"}}{\fldrslt text}}.\
}"#.as_bytes();
        let lines: Vec<String> = parse_rtf(source).unwrap().collect();
        assert_eq!(lines, vec!["This is commented-on text."]);
    }
}
