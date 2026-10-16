use super::*;

struct Multipart<'a> {
    client: Client,
    bucket: &'a str,
    key: &'a str,
    id: String,
    parts: Vec<CompletedPart>,
}
impl<'a> Multipart<'a> {
    async fn begin(
        settings: &'a S3Settings,
        key: &'a str,
        metadata: HashMap<String, String>,
    ) -> Result<Self> {
        let client = client(settings)?;
        let response = client
            .create_multipart_upload()
            .bucket(&settings.bucket)
            .key(key)
            .set_metadata(Some(metadata))
            .send()
            .await
            .context("could not start S3 multipart upload")?;
        let id = response
            .upload_id()
            .context("S3 did not return a multipart upload id")?
            .to_owned();
        Ok(Self {
            client,
            bucket: &settings.bucket,
            key,
            id,
            parts: Vec::new(),
        })
    }
    async fn send_parts(&mut self, path: &Path) -> Result<()> {
        let mut file = File::open(path).await?;
        loop {
            let mut bytes = Vec::with_capacity(MULTIPART_PART_SIZE);
            (&mut file)
                .take(MULTIPART_PART_SIZE as u64)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.is_empty() {
                return Ok(());
            }
            let number = self.parts.len() as i32 + 1;
            let result = self
                .client
                .upload_part()
                .bucket(self.bucket)
                .key(self.key)
                .upload_id(&self.id)
                .part_number(number)
                .body(ByteStream::from(bytes))
                .send()
                .await
                .with_context(|| format!("S3 multipart upload failed at part {number}"))?;
            self.parts.push(
                CompletedPart::builder()
                    .part_number(number)
                    .set_e_tag(result.e_tag().map(str::to_owned))
                    .build(),
            );
        }
    }
    async fn complete(&mut self) -> Result<()> {
        self.client
            .complete_multipart_upload()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(&self.id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(std::mem::take(&mut self.parts)))
                    .build(),
            )
            .send()
            .await
            .context("could not complete S3 multipart upload")?;
        Ok(())
    }
    async fn abort(&self) {
        let _ = self
            .client
            .abort_multipart_upload()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(&self.id)
            .send()
            .await;
    }
}
pub(super) async fn transfer(
    settings: &S3Settings,
    key: &str,
    path: &Path,
    metadata: HashMap<String, String>,
) -> Result<()> {
    let mut session = Multipart::begin(settings, key, metadata).await?;
    let result = async {
        session.send_parts(path).await?;
        session.complete().await
    }
    .await;
    if result.is_err() {
        session.abort().await;
    }
    result
}
