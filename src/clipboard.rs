use std::sync::{Arc, Mutex};

use anyhow::Result;

pub trait ClipboardProvider: Send {
    fn get_text(&mut self) -> Result<Option<String>>;
    fn set_text(&mut self, text: &str) -> Result<()>;
}

pub struct SystemClipboard {
    inner: arboard::Clipboard,
}

impl SystemClipboard {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: arboard::Clipboard::new()?,
        })
    }
}

impl ClipboardProvider for SystemClipboard {
    fn get_text(&mut self) -> Result<Option<String>> {
        match self.inner.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn set_text(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text.to_owned())?;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct MemoryClipboard {
    text: Arc<Mutex<Option<String>>>,
}

impl MemoryClipboard {
    pub fn set_memory_text(&self, text: impl Into<String>) {
        *self.text.lock().expect("memory clipboard poisoned") = Some(text.into());
    }

    pub fn memory_text(&self) -> Option<String> {
        self.text.lock().expect("memory clipboard poisoned").clone()
    }
}

impl ClipboardProvider for MemoryClipboard {
    fn get_text(&mut self) -> Result<Option<String>> {
        Ok(self.memory_text())
    }

    fn set_text(&mut self, text: &str) -> Result<()> {
        self.set_memory_text(text);
        Ok(())
    }
}
