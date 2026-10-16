//! Byte admission below gzip/tar, before parser-owned metadata allocations.
use super::{RecoveryConfig, ValidatedRecoveryPackage};
use crate::errors::AppError;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    time::{Duration, Instant},
};

pub(super) const EXTENSION_BYTES: u64 = 64 * 1024;

pub(super) fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}

pub(super) fn stream_error(error: io::Error) -> AppError {
    if error.get_ref().is_some_and(|inner| inner.is::<AppError>()) {
        *error.into_inner().unwrap().downcast::<AppError>().unwrap()
    } else {
        invalid(format!("Invalid recovery archive: {error}"))
    }
}

pub(super) fn check_deadline(started: Instant, limits: &RecoveryConfig) -> Result<(), AppError> {
    if started.elapsed() >= Duration::from_secs(limits.restore_deadline_seconds) {
        return Err(AppError::RecoveryDeadline(
            "Recovery validation deadline exceeded.".into(),
        ));
    }
    Ok(())
}

pub(super) struct CompressedInput<'a, R> {
    source: R,
    limits: &'a RecoveryConfig,
    started: Instant,
    digest: Sha256,
    consumed: u64,
}

impl<'a, R: Read> CompressedInput<'a, R> {
    pub(super) fn new(source: R, limits: &'a RecoveryConfig, started: Instant) -> Self {
        Self {
            source,
            limits,
            started,
            digest: Sha256::new(),
            consumed: 0,
        }
    }

    pub(super) fn finish(
        mut self,
        manifest: serde_json::Value,
    ) -> Result<ValidatedRecoveryPackage, AppError> {
        let mut buffer = [0; 64 * 1024];
        while self.read(&mut buffer).map_err(stream_error)? != 0 {}
        Ok(ValidatedRecoveryPackage {
            manifest,
            checksum_sha256: hex::encode(self.digest.finalize()),
            size_bytes: i64::try_from(self.consumed)
                .map_err(|_| invalid("Recovery package size overflowed."))?,
        })
    }
}

impl<R: Read> Read for CompressedInput<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check_deadline(self.started, self.limits).map_err(io::Error::other)?;
        let capacity = buffer.len().min(
            usize::try_from(
                self.limits
                    .max_upload_bytes
                    .saturating_sub(self.consumed)
                    .saturating_add(1),
            )
            .unwrap_or(usize::MAX),
        );
        let count = self.source.read(&mut buffer[..capacity])?;
        self.consumed = self
            .consumed
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other(invalid("Recovery package size overflowed.")))?;
        if self.consumed > self.limits.max_upload_bytes {
            return Err(io::Error::other(invalid(
                "Recovery package exceeds the compressed byte limit.",
            )));
        }
        self.digest.update(&buffer[..count]);
        Ok(count)
    }
}

pub(super) struct TarEnvelope<'a, R> {
    source: R,
    limits: &'a RecoveryConfig,
    started: Instant,
    header: [u8; 512],
    header_offset: usize,
    payload: u64,
    padding: u64,
    pax: Option<Vec<u8>>,
    pax_size: Option<u64>,
    extensions: usize,
    extension_bytes: u64,
    expanded: u64,
    tail: Option<u64>,
}

impl<'a, R: Read> TarEnvelope<'a, R> {
    pub(super) fn new(source: R, limits: &'a RecoveryConfig, started: Instant) -> Self {
        Self {
            source,
            limits,
            started,
            header: [0; 512],
            header_offset: 512,
            payload: 0,
            padding: 0,
            pax: None,
            pax_size: None,
            extensions: 0,
            extension_bytes: 0,
            expanded: 0,
            tail: None,
        }
    }

    pub(super) fn finish(mut self) -> Result<R, AppError> {
        let mut buffer = [0; 64 * 1024];
        while self.read(&mut buffer).map_err(stream_error)? != 0 {}
        Ok(self.source)
    }

    fn next_header(&mut self) -> io::Result<bool> {
        let mut filled = 0;
        while filled != self.header.len() {
            check_deadline(self.started, self.limits).map_err(io::Error::other)?;
            let count = self.source.read(&mut self.header[filled..])?;
            if count == 0 {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated tar header",
                ));
            }
            filled += count;
        }
        self.header_offset = 0;
        if self.header.iter().all(|byte| *byte == 0) {
            self.tail = Some(
                self.limits
                    .max_expanded_bytes
                    .saturating_sub(self.expanded)
                    .saturating_add(EXTENSION_BYTES),
            );
            return Ok(true);
        }
        let header = tar::Header::from_byte_slice(&self.header);
        let kind = header.entry_type();
        let recognized = header.as_gnu().is_some() || header.as_ustar().is_some();
        let extension = recognized
            && (kind.is_gnu_longname() || kind.is_gnu_longlink() || kind.is_pax_local_extensions());
        if !extension && !kind.is_file() && !kind.is_dir() {
            return Err(io::Error::other(invalid(format!(
                "Recovery package member has unsupported archive type {}.",
                kind.as_byte()
            ))));
        }
        let size = if extension {
            header.entry_size()?
        } else {
            self.pax_size.take().unwrap_or(header.entry_size()?)
        };
        if extension {
            self.extension_bytes = self
                .extension_bytes
                .checked_add(size)
                .ok_or_else(|| io::Error::other(invalid("Recovery extension size overflowed.")))?;
            self.extensions = self
                .extensions
                .checked_add(1)
                .ok_or_else(|| io::Error::other(invalid("Recovery extension count overflowed.")))?;
            if size > EXTENSION_BYTES.min(self.limits.max_member_bytes)
                || self.extensions > self.limits.max_members.saturating_mul(3)
                || self.extension_bytes > self.limits.max_manifest_bytes
            {
                return Err(io::Error::other(invalid(
                    "Recovery package exceeds the archive extension limit.",
                )));
            }
            if kind.is_pax_local_extensions() {
                self.pax = Some(Vec::new());
            }
        } else {
            if kind.is_dir() && size != 0 {
                return Err(io::Error::other(invalid(
                    "Recovery archive directory contains a payload.",
                )));
            }
            if size > self.limits.max_member_bytes {
                return Err(io::Error::other(invalid(
                    "Recovery package member exceeds its configured metadata or member size limit.",
                )));
            }
            self.expanded = self.expanded.checked_add(size).ok_or_else(|| {
                io::Error::other(invalid("Recovery package expanded size overflowed."))
            })?;
            if self.expanded > self.limits.max_expanded_bytes {
                return Err(io::Error::other(invalid(
                    "Recovery package exceeds the configured expanded byte limit.",
                )));
            }
        }
        self.payload = size;
        self.padding = (512 - size % 512) % 512;
        Ok(true)
    }

    fn finish_pax(&mut self) {
        if let Some(bytes) = self.pax.take() {
            self.pax_size = None;
            // Match tar's numeric override, including malformed-value fallback.
            for extension in tar::PaxExtensions::new(&bytes) {
                let Ok(extension) = extension else { break };
                if extension.key() == Ok("size") {
                    self.pax_size = extension.value().ok().and_then(|value| value.parse().ok());
                    break;
                }
            }
        }
    }
}

impl<R: Read> Read for TarEnvelope<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            check_deadline(self.started, self.limits).map_err(io::Error::other)?;
            if self.header_offset < 512 {
                let count = buffer.len().min(512 - self.header_offset);
                buffer[..count]
                    .copy_from_slice(&self.header[self.header_offset..self.header_offset + count]);
                self.header_offset += count;
                return Ok(count);
            }
            if self.payload != 0 {
                let capacity = buffer
                    .len()
                    .min(usize::try_from(self.payload).unwrap_or(usize::MAX));
                let count = self.source.read(&mut buffer[..capacity])?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated tar member",
                    ));
                }
                self.payload -= count as u64;
                if let Some(bytes) = &mut self.pax {
                    bytes.extend_from_slice(&buffer[..count]);
                }
                return Ok(count);
            }
            self.finish_pax();
            if self.padding != 0 {
                let capacity = buffer.len().min(self.padding as usize);
                let count = self.source.read(&mut buffer[..capacity])?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated tar padding",
                    ));
                }
                self.padding -= count as u64;
                return Ok(count);
            }
            if let Some(remaining) = &mut self.tail {
                let capacity = buffer
                    .len()
                    .min(usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX));
                let count = self.source.read(&mut buffer[..capacity])?;
                if count as u64 > *remaining {
                    return Err(io::Error::other(invalid(
                        "Recovery package exceeds the expanded byte limit after tar end.",
                    )));
                }
                *remaining -= count as u64;
                return Ok(count);
            }
            if !self.next_header()? {
                return Ok(0);
            }
        }
    }
}
