use crate::core::errors::DualBondError;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::FileExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;

/// Scrive un file a destinazione con supporto a offset 64 bit e senza
/// lo stallo di zero-fill NTFS (§0.3-D della spec): il file viene marcato
/// *sparse* con `FSCTL_SET_SPARSE` **prima** di `set_len`, così scrivere a
/// un offset lontano dall'inizio non forza NTFS ad azzerare fisicamente
/// tutto lo spazio intermedio.
pub struct FileWriter {
    file: File,
}

impl FileWriter {
    /// Crea (o tronca) il file in `path`, lo marca sparse e lo dimensiona
    /// a `total_size` byte. Ordine delle operazioni non negoziabile: sparse
    /// prima di `set_len`.
    pub fn create(path: &Path, total_size: u64) -> Result<Self, DualBondError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        set_sparse(&file)?;
        file.set_len(total_size)?;

        Ok(Self { file })
    }

    /// Riapre un file già creato da `create` senza troncarlo (resume,
    /// §4.5): i byte già durevoli non vanno mai toccati.
    pub fn open_existing(path: &Path) -> Result<Self, DualBondError> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    /// Scrive `buf` a `offset` (64 bit, nessun troncamento a u32/i32).
    pub fn write_at(&self, offset: u64, buf: &[u8]) -> Result<(), DualBondError> {
        let mut written = 0usize;
        while written < buf.len() {
            let n = self.file.seek_write(&buf[written..], offset + written as u64)?;
            if n == 0 {
                return Err(DualBondError::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "seek_write ha scritto 0 byte",
                )));
            }
            written += n;
        }
        Ok(())
    }

    /// Barriera di durabilità (§4.5): forza i byte scritti su disco.
    pub fn sync(&self) -> Result<(), DualBondError> {
        self.file.sync_data()?;
        Ok(())
    }
}

fn set_sparse(file: &File) -> Result<(), DualBondError> {
    let handle = file.as_raw_handle() as HANDLE;
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        return Err(crate::core::errors::win32_err("FSCTL_SET_SPARSE", code));
    }
    Ok(())
}

/// Dimensione realmente allocata su disco (non la dimensione logica): per
/// un file sparse con poche regioni scritte è molto minore di `set_len`.
/// Usata solo nei test per dimostrare che `create` non ha forzato NTFS a
/// zero-fillare fisicamente lo spazio.
#[cfg(test)]
fn allocated_size(path: &Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut high: u32 = 0;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    ((high as u64) << 32) | (low as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dualbond-test-{}-{}", std::process::id(), name));
        p
    }

    #[test]
    fn create_does_not_physically_allocate_full_size() {
        let path = temp_path("sparse-alloc.bin");
        let total_size = 200 * 1024 * 1024u64; // 200 MiB
        let _fw = FileWriter::create(&path, total_size).expect("create");

        let alloc = allocated_size(&path);
        assert!(
            alloc < 10 * 1024 * 1024,
            "file dovrebbe essere sparse (allocato={alloc}, dichiarato={total_size})"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), total_size);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_at_far_offset_completes_fast_and_is_correct() {
        let path = temp_path("far-offset.bin");
        let total_size = 300 * 1024 * 1024u64; // 300 MiB
        let fw = FileWriter::create(&path, total_size).expect("create");

        let offset = 290 * 1024 * 1024u64;
        let payload = b"dualbond-far-write-marker";

        let start = Instant::now();
        fw.write_at(offset, payload).expect("write_at");
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_secs_f64() < 5.0,
            "scrittura a offset lontano troppo lenta ({elapsed:?}): possibile zero-fill NTFS non evitato"
        );

        drop(fw);
        let mut readback = vec![0u8; payload.len()];
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = File::open(&path).unwrap();
            f.seek(SeekFrom::Start(offset)).unwrap();
            f.read_exact(&mut readback).unwrap();
        }
        assert_eq!(&readback[..], &payload[..]);

        let alloc = allocated_size(&path);
        assert!(
            alloc < 10 * 1024 * 1024,
            "regioni non scritte non dovrebbero essere state allocate fisicamente (allocato={alloc})"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_at_offset_beyond_4gib_does_not_truncate_to_u32() {
        let path = temp_path("beyond-4gib.bin");
        let total_size = 5u64 * 1024 * 1024 * 1024 + 4096; // ~5 GiB, sparse quindi economico
        let fw = FileWriter::create(&path, total_size).expect("create");

        let offset = 4u64 * 1024 * 1024 * 1024 + 123; // oltre 4 GiB
        let payload = b"beyond-4gib";
        fw.write_at(offset, payload).expect("write_at");

        drop(fw);
        let mut readback = vec![0u8; payload.len()];
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = File::open(&path).unwrap();
            f.seek(SeekFrom::Start(offset)).unwrap();
            f.read_exact(&mut readback).unwrap();
        }
        assert_eq!(&readback[..], &payload[..]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn create_zero_byte_file() {
        let path = temp_path("zero.bin");
        let fw = FileWriter::create(&path, 0).expect("create");
        drop(fw);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn create_one_byte_file_and_write_it() {
        let path = temp_path("one.bin");
        let fw = FileWriter::create(&path, 1).expect("create");
        fw.write_at(0, b"x").expect("write_at");
        drop(fw);
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        std::fs::remove_file(&path).ok();
    }
}
