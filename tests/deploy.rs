//! End-to-end deploy tests against a real FTP server running in Docker.
//!
//! These cover the parts `cargo test` otherwise cannot reach: the FTP client
//! (connect, LIST, RETR, STOR, RNFR/RNTO, DELE) and the deploy orchestration
//! in `sync::run`. They are `#[ignore]`d so the default test run stays
//! hermetic; CI runs them in a dedicated job.
//!
//!     cargo test --test deploy -- --ignored

mod common;

use common::{binary_blob, run, run_failing, with_server, LocalTree};

const STATE: &str = ".ftpsync-state.json";

/// A small site: text, a nested file, and a binary blob.
fn sample_site(tag: &str) -> LocalTree {
    let tree = LocalTree::new(tag);
    tree.write("index.html", b"<h1>hello</h1>\n");
    tree.write("assets/css/style.css", b"body{color:red}\n");
    tree.write("assets/app.bin", &binary_blob(128 * 1024, 42));
    tree.write("nested/deep/note.txt", b"deep\n");
    tree
}

#[test]
#[ignore = "requires docker"]
fn initial_deploy_uploads_everything_and_commits_state() {
    with_server(
        "initial_deploy_uploads_everything_and_commits_state",
        |server| {
            let tree = sample_site("initial");

            let output = run(&mut server.ftpsync(tree.path()));
            assert!(
                output.contains("4 to upload, 0 to delete"),
                "unexpected plan:\n{output}"
            );

            assert_eq!(
                server.files(),
                vec![
                    STATE.to_string(),
                    "assets/app.bin".to_string(),
                    "assets/css/style.css".to_string(),
                    "index.html".to_string(),
                    "nested/deep/note.txt".to_string(),
                ]
            );
            assert_eq!(server.read("index.html"), "<h1>hello</h1>\n");

            // The state file tracks the four uploads and never lists itself.
            let state = server.read(STATE);
            assert!(state.contains("\"nested/deep/note.txt\""), "state: {state}");
            assert!(!state.contains(STATE), "state file listed itself: {state}");

            // The advisory marker is cleaned up after a successful deploy.
            assert!(!server.exists(&format!("{STATE}.running")));
        },
    );
}

#[test]
#[ignore = "requires docker"]
fn binary_file_survives_the_roundtrip_byte_for_byte() {
    with_server(
        "binary_file_survives_the_roundtrip_byte_for_byte",
        |server| {
            let tree = sample_site("binary");
            run(&mut server.ftpsync(tree.path()));

            let expected = binary_blob(128 * 1024, 42);
            assert_eq!(server.read_bytes("assets/app.bin", tree.path()), expected);
        },
    );
}

#[test]
#[ignore = "requires docker"]
fn second_run_uploads_nothing() {
    with_server("second_run_uploads_nothing", |server| {
        let tree = sample_site("noop");
        run(&mut server.ftpsync(tree.path()));

        let output = run(&mut server.ftpsync(tree.path()));
        assert!(
            output.contains("Nothing to do"),
            "second run was not a no-op:\n{output}"
        );
        assert_eq!(server.files().len(), 5);
    });
}

#[test]
#[ignore = "requires docker"]
fn changed_file_is_reuploaded_and_orphan_is_deleted() {
    with_server(
        "changed_file_is_reuploaded_and_orphan_is_deleted",
        |server| {
            let tree = sample_site("mutate");
            run(&mut server.ftpsync(tree.path()));

            tree.write("index.html", b"<h1>updated</h1>\n");
            tree.remove("nested/deep/note.txt");

            let output = run(&mut server.ftpsync(tree.path()));
            assert!(
                output.contains("1 to upload, 1 to delete"),
                "unexpected plan:\n{output}"
            );
            assert_eq!(server.read("index.html"), "<h1>updated</h1>\n");
            assert!(!server.exists("nested/deep/note.txt"));
        },
    );
}

#[test]
#[ignore = "requires docker"]
fn no_delete_keeps_files_that_vanished_locally() {
    with_server("no_delete_keeps_files_that_vanished_locally", |server| {
        let tree = sample_site("no-delete");
        run(&mut server.ftpsync(tree.path()));

        tree.remove("nested/deep/note.txt");
        let output = run(server.ftpsync(tree.path()).arg("--no-delete"));

        assert!(
            output.contains("0 to upload, 0 to delete"),
            "unexpected plan:\n{output}"
        );
        assert!(server.exists("nested/deep/note.txt"));
    });
}

#[test]
#[ignore = "requires docker"]
fn dry_run_leaves_the_server_untouched() {
    with_server("dry_run_leaves_the_server_untouched", |server| {
        let tree = sample_site("dry-run");

        let output = run(server.ftpsync(tree.path()).arg("--dry-run"));
        assert!(output.contains("index.html"), "nothing planned:\n{output}");

        // Not even the state file may be written.
        assert!(
            server.files().is_empty(),
            "dry run wrote: {:?}",
            server.files()
        );
    });
}

#[test]
#[ignore = "requires docker"]
fn ftpignore_and_exclude_filter_the_upload() {
    with_server("ftpignore_and_exclude_filter_the_upload", |server| {
        let tree = LocalTree::new("filters");
        tree.write("index.html", b"keep\n");
        tree.write("debug.log", b"drop\n");
        tree.write("private/keys.txt", b"drop\n");
        tree.write(".ftpignore", b"*.log\n");

        run(server
            .ftpsync(tree.path())
            .args(["--exclude", "private/**"]));

        let files = server.files();
        assert!(files.contains(&"index.html".to_string()), "{files:?}");
        assert!(!files.iter().any(|f| f.ends_with(".log")), "{files:?}");
        assert!(
            !files.iter().any(|f| f.starts_with("private/")),
            "{files:?}"
        );
    });
}

#[test]
#[ignore = "requires docker"]
fn auto_init_adopts_identical_remote_files() {
    with_server("auto_init_adopts_identical_remote_files", |server| {
        let tree = LocalTree::new("auto-init");
        tree.write("index.html", b"same\n");

        // No state file, but the server already holds byte-identical content:
        // auto-init hashes it and the diff must come out empty.
        server.seed("index.html", "same\n");

        let output = run(&mut server.ftpsync(tree.path()));
        assert!(
            output.contains("auto-initializing"),
            "auto-init did not run:\n{output}"
        );
        assert!(
            output.contains("0 to upload, 0 to delete"),
            "identical file was not adopted:\n{output}"
        );
    });
}

#[test]
#[ignore = "requires docker"]
fn no_auto_init_treats_the_server_as_empty() {
    with_server("no_auto_init_treats_the_server_as_empty", |server| {
        let tree = LocalTree::new("no-auto-init");
        tree.write("index.html", b"same\n");
        server.seed("index.html", "same\n");

        let output = run(server.ftpsync(tree.path()).arg("--no-auto-init"));
        assert!(
            output.contains("treating server as empty"),
            "auto-init was not skipped:\n{output}"
        );
        assert!(
            output.contains("1 to upload, 0 to delete"),
            "unexpected plan:\n{output}"
        );
    });
}

#[test]
#[ignore = "requires docker"]
fn purge_empties_the_directory_but_keeps_it() {
    with_server("purge_empties_the_directory_but_keeps_it", |server| {
        let tree = sample_site("purge");
        server.seed("cache/stale.txt", "old\n");
        server.seed("cache/sub/deeper.txt", "old\n");

        run(server.ftpsync(tree.path()).args(["--purge", "cache"]));

        assert!(server.is_dir("cache"), "purge removed the directory itself");
        let leftovers: Vec<String> = server
            .files()
            .into_iter()
            .filter(|f| f.starts_with("cache/"))
            .collect();
        assert!(leftovers.is_empty(), "purge left: {leftovers:?}");
    });
}

#[test]
#[ignore = "requires docker"]
fn deploys_into_a_server_subdirectory() {
    with_server("deploys_into_a_server_subdirectory", |server| {
        let tree = LocalTree::new("subdir");
        tree.write("index.html", b"scoped\n");

        run(&mut server.ftpsync_at(tree.path(), "public"));

        assert_eq!(server.read("public/index.html"), "scoped\n");
        // The state file is scoped to --server-dir too.
        assert!(server.exists(&format!("public/{STATE}")));
        assert!(!server.exists(STATE));
    });
}

/// A deploy that dies part-way through must still record what it managed to
/// upload, otherwise the next run repeats the whole queue and a deploy over an
/// unreliable link never converges (issue #5).
///
/// The abort is provoked with a remote directory sitting on a path an upload
/// wants, which fails the rename the same way a dropped link fails a transfer,
/// but deterministically: `walker::discover` sorts ascending and the upload
/// workers pop from the back, so at `--concurrency 1` the files go out in
/// reverse alphabetical order and `00-blocked.html` is reached last.
#[test]
#[ignore = "requires docker"]
fn failed_deploy_still_commits_what_landed() {
    with_server("failed_deploy_still_commits_what_landed", |server| {
        let tree = LocalTree::new("partial");
        for i in 0..6 {
            tree.write(&format!("page{i}.html"), format!("page {i}\n").as_bytes());
        }
        tree.write("00-blocked.html", b"never lands\n");
        server.seed_dir("00-blocked.html");

        let output = run_failing(server.ftpsync(tree.path()).args(["--concurrency", "1"]));
        assert!(
            output.contains("State committed"),
            "the failed run committed no state:\n{output}"
        );

        // The six pages landed and are recorded; the blocked one is not.
        let state = server.read(STATE);
        for i in 0..6 {
            let page = format!("page{i}.html");
            assert_eq!(server.read(&page), format!("page {i}\n"));
            assert!(
                state.contains(&format!("\"{page}\"")),
                "missing {page}: {state}"
            );
        }
        assert!(
            !state.contains("00-blocked.html"),
            "state claims a file that never uploaded: {state}"
        );

        // The advisory marker is released even though the deploy failed.
        assert!(!server.exists(&format!("{STATE}.running")));

        // The point of it all: the next run retries only the leftover instead
        // of starting the whole queue over.
        let second = run_failing(&mut server.ftpsync(tree.path()));
        assert!(
            second.contains("1 to upload, 0 to delete"),
            "the deploy did not converge:\n{second}"
        );
    });
}
