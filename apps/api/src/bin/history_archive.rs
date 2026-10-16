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
    let mut input = Vec::new();
    std::fs::File::open(&arguments[3])?
        .take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 20 * 1024 * 1024 {
        bail!("archive exceeds free backup size limit");
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
