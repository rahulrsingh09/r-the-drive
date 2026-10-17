use anyhow::{bail, Context};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
};

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 5 || !matches!(arguments[1].as_str(), "encrypt" | "decrypt") {
        bail!("usage: history_archive encrypt|decrypt ORIGINAL_AGE_KEY_FILE INPUT OUTPUT");
    }
    let key = std::fs::read_to_string(&arguments[2]).context("reading original AGE key file")?;
    let mut source = std::fs::File::open(&arguments[3])?;
    let mut input = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    let limit = 20 * 1024 * 1024;
    loop {
        let available = buffer.len().min(limit + 1 - input.len());
        let count = match source.read(&mut buffer[..available]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        input.extend_from_slice(&buffer[..count]);
        if input.len() > limit {
            bail!("archive exceeds free backup size limit");
        }
    }
    let output = if arguments[1] == "encrypt" {
        panda_api::private_deployment::history_backup::encrypt(&input, key.trim())?
    } else {
        let identity: age::x25519::Identity = key
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid original AGE key"))?;
        let mut output = Vec::new();
        age::Decryptor::new(&input[..])?
            .decrypt(std::iter::once(&identity as &dyn age::Identity))?
            .read_to_end(&mut output)?;
        output
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&arguments[4])
        .context("creating a new private output file (existing files are never overwritten)")?;
    file.write_all(&output)?;
    file.sync_all()?;
    println!("Archive verified and written ({} bytes)", output.len());
    Ok(())
}

#[cfg(test)]
mod full_replacement_tests {
    use age::secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;

    fn run(
        mode: &str,
        key: &std::path::Path,
        input: &std::path::Path,
        output: &std::path::Path,
    ) -> std::process::Output {
        let binary = std::path::PathBuf::from(
            std::env::var("CARGO_TARGET_DIR").expect("built maintenance binary target"),
        )
        .join("debug/history_archive");
        std::process::Command::new(binary)
            .arg(mode)
            .arg(key)
            .arg(input)
            .arg(output)
            .output()
            .unwrap()
    }

    #[test]
    fn archive_round_trip_keeps_private_mode_and_refuses_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let key = directory.path().join("key");
        let input = directory.path().join("input");
        let encrypted = directory.path().join("encrypted");
        let output = directory.path().join("output");
        std::fs::write(
            &key,
            age::x25519::Identity::generate()
                .to_string()
                .expose_secret(),
        )
        .unwrap();
        std::fs::write(&input, b"synthetic archive\0binary\n").unwrap();
        assert!(run("encrypt", &key, &input, &encrypted).status.success());
        assert_eq!(
            std::fs::metadata(&encrypted).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let decoded = run("decrypt", &key, &encrypted, &output);
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&input).unwrap()
        );
        let again = run("decrypt", &key, &encrypted, &output);
        assert!(!again.status.success());
        assert!(
            String::from_utf8_lossy(&again.stderr).contains("existing files are never overwritten")
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&input).unwrap()
        );
    }

    #[test]
    fn malformed_or_oversized_archive_never_creates_output() {
        let directory = tempfile::tempdir().unwrap();
        let key = directory.path().join("key");
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        std::fs::write(
            &key,
            age::x25519::Identity::generate()
                .to_string()
                .expose_secret(),
        )
        .unwrap();
        std::fs::write(&input, b"malformed").unwrap();
        assert!(!run("decrypt", &key, &input, &output).status.success());
        assert!(!output.exists());
        std::fs::File::create(&input)
            .unwrap()
            .set_len(20 * 1024 * 1024 + 1)
            .unwrap();
        let oversized = run("decrypt", &key, &input, &output);
        assert!(!oversized.status.success());
        assert!(String::from_utf8_lossy(&oversized.stderr)
            .contains("archive exceeds free backup size limit"));
        assert!(!output.exists());
    }
}
