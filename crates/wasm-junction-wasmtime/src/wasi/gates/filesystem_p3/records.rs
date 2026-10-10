use wasmtime_wasi::p3::bindings::filesystem::types::{
    DescriptorStat, DescriptorType, DirectoryEntry, MetadataHashValue, NewTimestamp,
};

use super::{CallError, FromVal, ToVal, Val, shape};
use crate::engine::StoreData;

impl ToVal for NewTimestamp {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::NoChange => ("no-change", None),
            Self::Now => ("now", None),
            Self::Timestamp(value) => ("timestamp", Some(Box::new(value.to_val()))),
        };
        Val::Variant {
            case: case.to_owned(),
            value,
        }
    }
}

impl FromVal for NewTimestamp {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Variant { case, value: None } if case == "no-change" => Ok(Self::NoChange),
            Val::Variant { case, value: None } if case == "now" => Ok(Self::Now),
            Val::Variant {
                case,
                value: Some(value),
            } if case == "timestamp" => {
                wasmtime_wasi::p3::bindings::clocks::system_clock::Instant::from_val(*value)
                    .map(Self::Timestamp)
            }
            _ => Err(shape("new-timestamp")),
        }
    }
}

impl ToVal for DescriptorStat {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("type".to_owned(), self.type_.to_val()),
            ("link-count".to_owned(), self.link_count.to_val()),
            ("size".to_owned(), self.size.to_val()),
            (
                "data-access-timestamp".to_owned(),
                self.data_access_timestamp.to_val(),
            ),
            (
                "data-modification-timestamp".to_owned(),
                self.data_modification_timestamp.to_val(),
            ),
            (
                "status-change-timestamp".to_owned(),
                self.status_change_timestamp.to_val(),
            ),
        ])
    }
}

impl FromVal for DescriptorStat {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("descriptor-stat"));
        };
        let [
            (_, type_),
            (_, link_count),
            (_, size),
            (_, accessed),
            (_, modified),
            (_, changed),
        ] = <[_; 6]>::try_from(fields).map_err(|_| shape("descriptor-stat fields"))?;
        Ok(Self {
            type_: DescriptorType::from_val(type_)?,
            link_count: u64::from_val(link_count)?,
            size: u64::from_val(size)?,
            data_access_timestamp: Option::from_val(accessed)?,
            data_modification_timestamp: Option::from_val(modified)?,
            status_change_timestamp: Option::from_val(changed)?,
        })
    }
}

impl ToVal for MetadataHashValue {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("lower".to_owned(), self.lower.to_val()),
            ("upper".to_owned(), self.upper.to_val()),
        ])
    }
}

impl FromVal for MetadataHashValue {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("metadata-hash-value"));
        };
        let [(_, lower), (_, upper)] =
            <[_; 2]>::try_from(fields).map_err(|_| shape("metadata-hash-value fields"))?;
        Ok(Self {
            lower: u64::from_val(lower)?,
            upper: u64::from_val(upper)?,
        })
    }
}

impl ToVal for DirectoryEntry {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("type".to_owned(), self.type_.to_val()),
            ("name".to_owned(), self.name.to_val()),
        ])
    }
}

impl FromVal for DirectoryEntry {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("directory-entry"));
        };
        let [(_, type_), (_, name)] =
            <[_; 2]>::try_from(fields).map_err(|_| shape("directory-entry fields"))?;
        Ok(Self {
            type_: DescriptorType::from_val(type_)?,
            name: String::from_val(name)?,
        })
    }
}

impl crate::stream_values::StreamValue for DirectoryEntry {
    fn into_val(
        self,
        _ty: Option<&wasmtime::component::Type>,
        _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        Ok(Some(self.to_val()))
    }

    fn from_val(
        value: Option<Val>,
        _ty: Option<&wasmtime::component::Type>,
        _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        <Self as FromVal>::from_val(
            value.ok_or_else(|| wasmtime::Error::msg("directory entry cannot be unit"))?,
        )
        .map_err(wasmtime::Error::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime_wasi::p3::bindings::clocks::system_clock::Instant;

    #[test]
    fn filesystem_records_round_trip() {
        let timestamp = NewTimestamp::Timestamp(Instant {
            seconds: -1,
            nanoseconds: 7,
        });
        assert!(matches!(
            NewTimestamp::from_val(timestamp.to_val()),
            Ok(NewTimestamp::Timestamp(Instant {
                seconds: -1,
                nanoseconds: 7
            }))
        ));

        let hash = MetadataHashValue { lower: 3, upper: 5 };
        let decoded = MetadataHashValue::from_val(hash.to_val()).unwrap();
        assert_eq!((decoded.lower, decoded.upper), (3, 5));

        let entry = DirectoryEntry {
            type_: DescriptorType::RegularFile,
            name: "note.txt".to_owned(),
        };
        let decoded = <DirectoryEntry as FromVal>::from_val(entry.to_val()).unwrap();
        assert!(matches!(decoded.type_, DescriptorType::RegularFile));
        assert_eq!(decoded.name, "note.txt");
    }
}
