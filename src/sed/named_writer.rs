// An abstraction for output files created on entry and flushed on exit
//
// SPDX-License-Identifier: MIT
// Copyright (c) 2025 Diomidis Spinellis
//
// This file is part of the uutils sed package.
// It is licensed under the MIT License.
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

use crate::sed::error_handling::{ScriptLocation, runtime_error};

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::rc::Rc;

use uucore::display::Quotable;
use uucore::error::UResult;

thread_local! {
    /// Global list of all writers that should be flushed at shutdown
    static FLUSH_LIST: RefCell<Vec<Rc<RefCell<NamedWriter>>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug)]
/// Writer that tracks its file name for better error messages
pub struct NamedWriter {
    pub path: PathBuf,
    writer: BufWriter<File>,
    location: ScriptLocation,
    pending_separator: bool, // The last line written lacked its separator
}

impl NamedWriter {
    /// Create a new writer, truncate the file, and register it for flushing.
    pub fn new(path: PathBuf, location: ScriptLocation) -> UResult<Rc<RefCell<Self>>> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| {
                runtime_error::<()>(&location, format!("creating file {}: {}", path.quote(), e))
                    .unwrap_err()
            })?;

        let writer = Rc::new(RefCell::new(NamedWriter {
            path,
            writer: BufWriter::new(file),
            location,
            pending_separator: false,
        }));

        FLUSH_LIST.with(|list| list.borrow_mut().push(Rc::clone(&writer)));
        Ok(writer)
    }

    /// Write String to the file, possibly with a newline, returning errors.
    pub fn write_line(&mut self, line: &str, newline: bool) -> UResult<()> {
        self.write_line_bytes(line.as_bytes(), b'\n', newline)
    }

    /// Write bytes to the file, possibly followed by `separator`, returning
    /// errors. A missing separator is written before any further line.
    pub fn write_line_bytes(
        &mut self,
        line: &[u8],
        separator: u8,
        terminated: bool,
    ) -> UResult<()> {
        if line.is_empty() && !terminated {
            return Ok(());
        }
        let pending = std::mem::replace(&mut self.pending_separator, !terminated);
        let writer = &mut self.writer;
        (|| -> io::Result<()> {
            if pending {
                writer.write_all(&[separator])?;
            }
            writer.write_all(line)?;
            if terminated {
                writer.write_all(&[separator])?;
            }
            Ok(())
        })()
        .map_err(|e| {
            runtime_error::<()>(
                &self.location,
                format!("writing to file {}: {e}", self.path.quote()),
            )
            .unwrap_err()
        })
    }

    /// Flush the writer, returning a descriptive error.
    pub fn flush(&mut self) -> UResult<()> {
        self.writer.flush().map_err(|e| {
            runtime_error::<()>(
                &self.location,
                format!("writing to file {}: {}", self.path.quote(), e),
            )
            .unwrap_err()
        })
    }
}

/// Flush buffered content to the file, returning descriptive errors.
pub fn flush_all() -> UResult<()> {
    FLUSH_LIST.with(|cell| {
        for handle in cell.borrow().iter() {
            handle.borrow_mut().flush()?;
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_write_line_bytes_appends_newline() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        let writer = NamedWriter::new(path.clone(), ScriptLocation::default()).unwrap();

        writer
            .borrow_mut()
            .write_line_bytes(b"a\xE9", b'\n', true)
            .unwrap();
        writer.borrow_mut().flush().unwrap();

        assert_eq!(fs::read(path).unwrap(), b"a\xE9\n");
    }

    #[test]
    fn test_write_line_bytes_appends_no_newline() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        let writer = NamedWriter::new(path.clone(), ScriptLocation::default()).unwrap();

        writer
            .borrow_mut()
            .write_line_bytes(b"a\xE9", b'\n', false)
            .unwrap();
        writer.borrow_mut().flush().unwrap();

        assert_eq!(fs::read(path).unwrap(), b"a\xE9");
    }
}
