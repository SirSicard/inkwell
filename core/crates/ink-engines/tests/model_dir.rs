//! The model directory: what a download still needs, and removing a row's files.

mod common;

use common::{Scratch, install, row};
use ink_core::Job;

#[test]
fn bytes_on_disk_count_finished_files_and_part_files_up_to_their_size() {
    let s = Scratch::new("bytes");
    let dir = s.model_dir();
    let r = row("synthetic-asr", &[(Job::DictationFinal, 5.0)]);
    let size = r.files[0].size;
    assert_eq!(dir.bytes_on_disk(&r), 0, "nothing there");
    let part = dir.part_path(&r, &r.files[0]);
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, vec![0u8; 3]).unwrap();
    assert_eq!(dir.bytes_on_disk(&r), 3, "a part file");
    std::fs::write(&part, vec![0u8; size as usize + 10]).unwrap();
    assert_eq!(dir.bytes_on_disk(&r), size, "never more than the file");
    install(&dir, &r);
    assert_eq!(dir.bytes_on_disk(&r), size, "installed");
}

#[test]
fn removing_a_row_deletes_every_revision_part_and_marker_and_nothing_else() {
    let s = Scratch::new("remove");
    let dir = s.model_dir();
    let r = row("synthetic-asr", &[(Job::DictationFinal, 5.0)]);
    let other = row("synthetic-other", &[(Job::DictationFinal, 6.0)]);
    install(&dir, &r);
    install(&dir, &other);
    // Another revision's leftovers and a part file under the row's own id.
    let stale = dir.root().join(&r.id).join("feedfacefeed");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("weights.bin.part"), b"x").unwrap();
    assert!(dir.is_installed(&r));

    dir.remove_row(&r).unwrap();
    assert!(!dir.is_installed(&r));
    assert!(!dir.root().join(&r.id).exists());
    assert!(dir.is_installed(&other), "another row is untouched");
    // Nothing there is fine.
    dir.remove_row(&r).unwrap();
}

#[test]
fn removing_a_row_refuses_an_invalid_row_and_deletes_nothing() {
    let s = Scratch::new("remove-invalid");
    let dir = s.model_dir();
    let r = row("synthetic-asr", &[(Job::DictationFinal, 5.0)]);
    install(&dir, &r);
    let mut climbing = r.clone();
    climbing.id = "..".into();
    assert!(dir.remove_row(&climbing).is_err());
    assert!(dir.is_installed(&r));
    assert!(dir.root().exists());
}

#[cfg(unix)]
#[test]
fn removing_a_row_whose_directory_is_a_link_deletes_nothing() {
    let s = Scratch::new("remove-link");
    let dir = s.model_dir();
    let r = row("synthetic-asr", &[(Job::DictationFinal, 5.0)]);
    // The row's directory is a link to a directory outside the root, holding a file.
    let outside = s.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    std::fs::create_dir_all(dir.root()).unwrap();
    std::os::unix::fs::symlink(&outside, dir.root().join(&r.id)).unwrap();

    assert!(dir.remove_row(&r).is_err());
    assert!(
        outside.join("keep.txt").exists(),
        "nothing outside the root is deleted"
    );
}

#[cfg(unix)]
#[test]
fn a_link_inside_a_row_is_removed_never_followed() {
    let s = Scratch::new("remove-inner-link");
    let dir = s.model_dir();
    let r = row("synthetic-asr", &[(Job::DictationFinal, 5.0)]);
    install(&dir, &r);
    let outside = s.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, dir.row_dir(&r).join("elsewhere")).unwrap();

    dir.remove_row(&r).unwrap();
    assert!(!dir.root().join(&r.id).exists());
    assert!(outside.join("keep.txt").exists());
}
