wit_bindgen::generate!({
    path: "../../wit",
    world: "notes-tools-component",
});

use wasip3::filesystem::types::{
    Descriptor, DescriptorFlags, DescriptorType, OpenFlags, PathFlags,
};
use wasip3::wit_stream;

struct Component;

impl exports::example::assistant::notes_tools::Guest for Component {
    async fn list_notes() -> Result<Vec<String>, String> {
        let root = notes_directory()?;
        let mut notes = Vec::new();
        for entry in entries(&root).await? {
            match entry.type_ {
                DescriptorType::RegularFile => notes.push(entry.name),
                DescriptorType::Directory => {
                    let directory = open(
                        &root,
                        &entry.name,
                        OpenFlags::DIRECTORY,
                        DescriptorFlags::READ,
                    )
                    .await?;
                    for child in entries(&directory).await? {
                        if matches!(child.type_, DescriptorType::RegularFile) {
                            notes.push(format!("{}/{}", entry.name, child.name));
                        }
                    }
                }
                _ => {}
            }
        }
        notes.sort();
        Ok(notes)
    }

    async fn read_note(path: String) -> Result<String, String> {
        let file = open(
            &notes_directory()?,
            &path,
            OpenFlags::empty(),
            DescriptorFlags::READ,
        )
        .await?;
        let (bytes, completion) = file.read_via_stream(0);
        let bytes = bytes.collect().await;
        completion.await.map_err(error)?;
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }

    async fn write_note(path: String, text: String) -> Result<(), String> {
        let file = open(
            &notes_directory()?,
            &path,
            OpenFlags::CREATE | OpenFlags::TRUNCATE,
            DescriptorFlags::WRITE,
        )
        .await?;
        let (mut writer, reader) = wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(text.into_bytes()).await;
        });
        file.write_via_stream(reader, 0).await.map_err(error)
    }
}

fn notes_directory() -> Result<Descriptor, String> {
    wasip3::filesystem::preopens::get_directories()
        .into_iter()
        .find(|(_, path)| path == "/notes")
        .map(|(directory, _)| directory)
        .ok_or_else(|| "the /notes directory is not preopened".into())
}

async fn entries(
    directory: &Descriptor,
) -> Result<Vec<wasip3::filesystem::types::DirectoryEntry>, String> {
    let (entries, completion) = directory.read_directory();
    let entries = entries.collect().await;
    completion.await.map_err(error)?;
    Ok(entries)
}

async fn open(
    directory: &Descriptor,
    path: &str,
    open_flags: OpenFlags,
    descriptor_flags: DescriptorFlags,
) -> Result<Descriptor, String> {
    directory
        .open_at(
            PathFlags::empty(),
            path.into(),
            open_flags,
            descriptor_flags,
        )
        .await
        .map_err(error)
}

fn error(error: wasip3::filesystem::types::ErrorCode) -> String {
    plain_error(&format!("{error:?}"))
}

fn plain_error(debug: &str) -> String {
    let name = debug
        .strip_prefix("ErrorCode::")
        .unwrap_or(debug)
        .split('(')
        .next()
        .unwrap_or(debug);
    name.chars()
        .enumerate()
        .flat_map(|(index, character)| {
            (index > 0 && character.is_uppercase())
                .then_some(' ')
                .into_iter()
                .chain(character.to_lowercase())
        })
        .collect()
}

export!(Component);
