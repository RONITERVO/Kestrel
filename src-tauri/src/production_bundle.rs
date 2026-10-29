//! Format-neutral delivery. No output type, generator, platform or upload workflow is required.
use crate::{media_composition, media_delivery::probe, production_artifacts::resolve, AppState};
use kestrel_app_core::{
    ProductionArtifactFormat, ProductionBundle, ProductionBundleRequest, ProductionImageFit,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn read_record(path: &Path) -> Result<Value, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
        return Err("Bundle record exceeds 4 MiB.".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn get(root: &Path, id: &str) -> Result<Value, String> {
    uuid::Uuid::parse_str(id).map_err(|_| {
        "Bundle id must be a UUID from list_production_bundles or export_production_bundle."
    })?;
    let parent = root.join("producer/bundles");
    let complete = parent.join(id).join("manifest.json");
    if complete.is_file() {
        return read_record(&complete);
    }
    let path = parent.join(format!(".partial-{id}")).join("request.json");
    if !path.is_file() {
        return Err("Bundle not found. Use its bundle id, not the caller's jobId.".into());
    }
    Ok(
        json!({"id":id,"status":"incomplete","request":read_record(&path)?,"detail":"Partial native work is preserved and will not resume automatically."}),
    )
}

pub fn list(root: &Path) -> Result<Vec<kestrel_app_core::ProductionBundleSummary>, String> {
    let parent = root.join("producer/bundles");
    if !parent.is_dir() {
        return Ok(vec![]);
    }
    let mut entries = fs::read_dir(&parent)
        .map_err(|e| e.to_string())?
        .take(10_001)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if entries.len() > 10_000 {
        return Err(
            "Bundle catalog exceeds 10,000 entries; archive older outputs before listing.".into(),
        );
    }
    entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.metadata().and_then(|m| m.modified()).ok()));
    let mut results = Vec::new();
    for entry in entries.into_iter().take(200) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = name.strip_prefix(".partial-").unwrap_or(&name);
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let record = get(root, id)?;
        results.push(kestrel_app_core::ProductionBundleSummary {
            id: id.into(),
            title: record
                .get("title")
                .or_else(|| record.pointer("/request/title"))
                .and_then(Value::as_str)
                .unwrap_or("Production bundle")
                .into(),
            status: if record["status"] == "incomplete" {
                "incomplete"
            } else {
                "complete"
            }
            .into(),
            native_directory: entry.path().to_string_lossy().into_owned(),
        });
    }
    Ok(results)
}

fn safe_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 240
        || name.split('/').count() > 8
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"\\|?*".contains(c))
    {
        return Err("Bundle filenames need 1–240 characters, at most eight components and forward slashes; absolute paths, wildcards and Windows special characters are not allowed.".into());
    }
    for part in name.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!(
                "Unsafe bundle filename component: {part}. Choose a regular relative filename."
            ));
        }
    }
    Ok(())
}

fn validate(request: &ProductionBundleRequest) -> Result<(), String> {
    if request.title.trim().is_empty()
        || request.title.chars().count() > 200
        || request.description.len() > 32_000
        || request.artifacts.len() + request.files.len() == 0
        || request.artifacts.len() + request.files.len() > 128
    {
        return Err("A bundle needs a title, at most 32,000 description bytes, and 1–128 selected artifacts or text files.".into());
    }
    if !Path::new(&request.output_directory).is_absolute()
        || request.output_directory.contains(['*', '?'])
    {
        return Err("Choose an absolute outputDirectory without wildcards.".into());
    }
    let mut names = HashSet::from(["manifest.json".to_string(), "request.json".to_string()]);
    for name in request
        .artifacts
        .iter()
        .map(|a| a.filename.as_str())
        .chain(request.files.iter().map(|f| f.filename.as_str()))
    {
        safe_name(name)?;
        let key = name.to_lowercase();
        if names.iter().any(|prior| {
            prior == &key
                || key.starts_with(&format!("{prior}/"))
                || prior.starts_with(&format!("{key}/"))
        }) {
            return Err(format!(
                "Bundle filename collides with another file or reserved manifest: {name}"
            ));
        }
        names.insert(key);
    }
    for file in &request.files {
        if file.text.len() > 1024 * 1024 {
            return Err("A bundle text file exceeds 1 MiB.".into());
        }
    }
    if request.files.iter().map(|f| f.text.len()).sum::<usize>() > 2 * 1024 * 1024 {
        return Err("Bundle authored text exceeds 2 MiB in total; import larger source files as attachments.".into());
    }
    for item in &request.artifacts {
        if item.role.len() > 1000 {
            return Err("An artifact role exceeds 1,000 bytes.".into());
        }
        if let ProductionArtifactFormat::JpegImage { width, height, .. } = item.format {
            if !(32..=4096).contains(&width)
                || !(32..=4096).contains(&height)
                || !matches!(
                    Path::new(&item.filename)
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("jpg" | "jpeg")
                )
            {
                return Err(
                    "JPEG conversions need width/height 32–4096 and a .jpg or .jpeg filename."
                        .into(),
                );
            }
        }
    }
    Ok(())
}

async fn copy(source: &Path, destination: &Path, cancel: &CancellationToken) -> Result<(), String> {
    let mut input = tokio::fs::File::open(source)
        .await
        .map_err(|e| e.to_string())?;
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|e| e.to_string())?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err("Bundle stopped; partial native files are preserved and nothing will resume automatically.".into());
        }
        let count = input.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .await
            .map_err(|e| e.to_string())?;
    }
    output.sync_all().await.map_err(|e| e.to_string())
}

pub async fn export(
    state: &AppState,
    request: ProductionBundleRequest,
    cancel: CancellationToken,
) -> Result<ProductionBundle, String> {
    validate(&request)?;
    let id = uuid::Uuid::new_v4().to_string();
    let result = assemble(state, &request, &cancel, &id).await;
    result.map_err(|error| {
        let parent = state.store.root().join("producer/bundles");
        if parent.join(&id).exists() || parent.join(format!(".partial-{id}")).exists() {
            format!(
                "{error} Preserved bundle id: {id}. Inspect get_production_bundle with this id."
            )
        } else {
            format!("{error} No bundle was created.")
        }
    })
}

async fn assemble(
    state: &AppState,
    request: &ProductionBundleRequest,
    cancel: &CancellationToken,
    id: &str,
) -> Result<ProductionBundle, String> {
    let sources = request
        .artifacts
        .iter()
        .map(|a| resolve(state, &a.source))
        .collect::<Result<Vec<_>, _>>()?;
    let total_bytes = sources.iter().try_fold(0_u64, |sum, source| {
        fs::metadata(&source.path)
            .map(|m| sum.saturating_add(m.len()))
            .map_err(|e| e.to_string())
    })?;
    if total_bytes > 32 * 1024 * 1024 * 1024 {
        return Err(
            "Bundle inputs exceed 32 GiB; split the selection into smaller bundles.".into(),
        );
    }
    let settings = state.research_settings.load().map_err(|e| e.to_string())?;
    let parent = state.store.root().join("producer/bundles");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let stage = parent.join(format!(".partial-{id}"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    fs::write(
        stage.join("request.json"),
        serde_json::to_vec_pretty(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut receipts: Vec<Value> = Vec::new();
    for (index, (item, source)) in request.artifacts.iter().zip(&sources).enumerate() {
        if cancel.is_cancelled() {
            return Err("Bundle stopped before the next artifact.".into());
        }
        state.control.observe("media-composition",json!({"title":format!("Bundling artifact {} of {}",index+1,sources.len()),"detail":item.filename}))?;
        let target = stage.join(&item.filename);
        fs::create_dir_all(target.parent().ok_or("Bundle target has no parent.")?)
            .map_err(|e| e.to_string())?;
        let mut conversion_args: Vec<OsString> = Vec::new();
        match item.format {
            ProductionArtifactFormat::Original => {
                if target.extension().map(|v| v.to_ascii_lowercase())
                    != source.path.extension().map(|v| v.to_ascii_lowercase())
                {
                    return Err(format!("Original copies preserve their file format. {} needs the source's .{} extension.",item.filename,source.path.extension().unwrap_or_default().to_string_lossy()));
                }
                copy(&source.path, &target, cancel).await?;
            }
            ProductionArtifactFormat::JpegImage { width, height, fit } => {
                if !source.still {
                    return Err("JPEG conversion currently requires an image artifact; video frame extraction is a separate operation.".into());
                }
                let filter = match fit {
                    ProductionImageFit::Contain => format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"),
                    ProductionImageFit::Cover => format!("scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"),
                };
                conversion_args = ["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"]
                    .into_iter()
                    .map(Into::into)
                    .collect();
                conversion_args.extend([
                    source.path.as_os_str().into(),
                    "-vf".into(),
                    filter.into(),
                    "-frames:v".into(),
                    "1".into(),
                    "-q:v".into(),
                    "2".into(),
                    target.as_os_str().into(),
                ]);
                media_composition::run(Path::new(&settings.ffmpeg_path), &conversion_args, cancel)
                    .await?;
                let info = probe(Path::new(&settings.ffprobe_path), &target).await?;
                if !info["streams"].as_array().is_some_and(|streams| {
                    streams.iter().any(|s| {
                        s["codec_name"] == "mjpeg" && s["width"] == width && s["height"] == height
                    })
                }) {
                    return Err("Converted image failed dimensions/format verification.".into());
                }
            }
        }
        let (bytes, hash) = crate::studio::hash_file(&target).map_err(|e| e.to_string())?;
        if matches!(item.format, ProductionArtifactFormat::Original) && hash != source.hash {
            return Err(
                "An artifact changed during copy; the incomplete bundle was preserved.".into(),
            );
        }
        receipts.push(json!({"filename":item.filename,"role":item.role,"source":item.source,"sourcePath":source.path,"sourceSha256":source.hash,"bytes":bytes,"sha256":hash,"format":item.format,"conversionArguments":conversion_args.iter().map(|a|a.to_string_lossy()).collect::<Vec<_>>()}));
    }
    for file in &request.files {
        if cancel.is_cancelled() {
            return Err("Bundle stopped before writing the next text file.".into());
        }
        let target = stage.join(&file.filename);
        fs::create_dir_all(target.parent().ok_or("Bundle text file has no parent.")?)
            .map_err(|e| e.to_string())?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut output, file.text.as_bytes()).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        let (bytes, hash) = crate::studio::hash_file(&target).map_err(|e| e.to_string())?;
        receipts.push(json!({"filename":file.filename,"source":{"kind":"producerText"},"bytes":bytes,"sha256":hash}));
    }
    let manifest = json!({"schemaVersion":1,"id":id,"title":request.title,"description":request.description,"files":receipts,"createdAt":crate::app_control::now(),"uploadPerformed":false});
    fs::write(
        stage.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let native = parent.join(id);
    fs::rename(&stage, &native).map_err(|e| e.to_string())?;
    fs::create_dir_all(&request.output_directory).map_err(|e| e.to_string())?;
    let delivery_stage =
        PathBuf::from(&request.output_directory).join(format!(".kestrel-bundle-{id}"));
    let delivery = PathBuf::from(&request.output_directory).join(format!("bundle-{id}"));
    fs::create_dir(&delivery_stage).map_err(|e| e.to_string())?;
    let mut names = receipts
        .iter()
        .filter_map(|r| r["filename"].as_str())
        .map(String::from)
        .collect::<Vec<_>>();
    names.extend(["request.json".into(), "manifest.json".into()]);
    for name in &names {
        let target = delivery_stage.join(name);
        fs::create_dir_all(target.parent().ok_or("Delivery file has no parent.")?)
            .map_err(|e| e.to_string())?;
        copy(&native.join(name), &target, cancel).await?;
    }
    fs::rename(&delivery_stage, &delivery).map_err(|e| e.to_string())?;
    Ok(ProductionBundle {
        id: id.into(),
        directory: delivery.to_string_lossy().into_owned(),
        manifest_path: delivery
            .join("manifest.json")
            .to_string_lossy()
            .into_owned(),
        files: names
            .iter()
            .map(|name| delivery.join(name).to_string_lossy().into_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::ProductionTextFile;
    #[test]
    fn bundle_paths_reject_escape_devices_aliases_and_duplicate_names() {
        for name in [
            "../outside.txt",
            "/absolute.txt",
            "C:/absolute.txt",
            "dir\\file.txt",
            "a:b.txt",
            "NUL.txt",
            "folder/COM1.png",
            "x/../a",
            "trailing. ",
        ] {
            assert!(safe_name(name).is_err(), "{name}");
        }
        assert!(safe_name("assets/front-view.png").is_ok());
        let mut request = ProductionBundleRequest {
            job_id: "test".into(),
            title: "App assets".into(),
            description: String::new(),
            artifacts: vec![],
            files: vec![ProductionTextFile {
                filename: "index.html".into(),
                text: "<canvas></canvas>".into(),
            }],
            output_directory: std::env::temp_dir().to_string_lossy().into_owned(),
        };
        assert!(validate(&request).is_ok());
        request.files.push(ProductionTextFile {
            filename: "INDEX.HTML".into(),
            text: String::new(),
        });
        assert!(validate(&request).is_err());
        request.files[1].filename = "manifest.json".into();
        assert!(validate(&request).is_err());
        request.files[1].filename = "index.html/child.txt".into();
        assert!(validate(&request).is_err());
    }
}
