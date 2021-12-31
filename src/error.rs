use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};

#[macro_export]
macro_rules! err {
    () => {
        |e| $crate::Error::new(file!(), line!(), None, e)
    };
    ($($args:tt)+) => {
        |e| $crate::Error::new(file!(), line!(), format!($($args)*), e)
    };
}

pub type Result<T> = std::result::Result<T, Error>;

pub trait WithContext {
    fn ctx(self, name: impl Display, value: impl Display) -> Self;
}

impl<T> WithContext for Result<T> {
    fn ctx(self, name: impl Display, value: impl Display) -> Self {
        match self {
            Ok(v) => Ok(v),
            Err(mut e) => {
                e.ctx(name, value);
                Err(e)
            }
        }
    }
}

#[derive(Debug)]
pub struct Error(Box<Inner>);

impl Error {
    pub fn new(
        file: &'static str,
        line: u32,
        msg: impl Into<Option<String>>,
        source: impl StdError + Send + 'static,
    ) -> Self {
        Self(Box::new(Inner::new(file, line, msg, source)))
    }

    pub fn ctx(&mut self, name: impl Display, value: impl Display) {
        self.0.ctx(name, value);
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

#[derive(Debug)]
struct Inner {
    file: &'static str,
    line: u32,
    msg: Option<String>,
    context: HashMap<String, String>,
    source: Box<dyn StdError + Send>,
}

impl Inner {
    fn new(
        file: &'static str,
        line: u32,
        msg: impl Into<Option<String>>,
        source: impl StdError + Send + 'static,
    ) -> Self {
        Self {
            file,
            line,
            msg: msg.into(),
            context: HashMap::new(),
            source: Box::new(source),
        }
    }

    fn ctx(&mut self, name: impl Display, value: impl Display) {
        self.context
            .insert(format!("{}", name), format!("{}", value));
    }
}

impl Display for Inner {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let ctx = if self.context.is_empty() {
            "".to_string()
        } else {
            let mut s = String::with_capacity(64);
            for (k, v) in &self.context {
                s.push_str(&format!("{}: {}, ", k, v));
            }
            format!(" {{ {} }}", &s[..s.len() - 2])
        };
        let msg = match self.msg {
            Some(ref msg) => format!(" {}: ", msg),
            None => " ".to_string(),
        };
        write!(
            f,
            "[{}:{}]{}{}{}",
            self.file, self.line, msg, self.source, ctx
        )
    }
}

impl StdError for Inner {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.source)
    }
}
