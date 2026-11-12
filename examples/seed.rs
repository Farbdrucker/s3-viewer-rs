//! Development helper: uploads a local folder into a bucket.
//!
//! cargo run --example seed -- <endpoint> <access key> <secret key> <bucket> <folder> [prefix]

use s3_viewer::s3::{CancelFlag, Session, SignatureVersion, validate_settings};
use s3_viewer::transfer::folder_upload_plan;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [endpoint, access, secret, bucket, folder, rest @ ..] = args.as_slice() else {
        eprintln!("usage: seed <endpoint> <access key> <secret key> <bucket> <folder> [prefix]");
        std::process::exit(2);
    };
    let prefix = rest.first().map(String::as_str).unwrap_or("");
    let settings = validate_settings(endpoint, access, secret, "us-east-1", "", true, true, SignatureVersion::V4)
        .unwrap_or_else(|error| panic!("{error}"));
    let session = Session::new(settings).unwrap_or_else(|error| panic!("{error}"));
    let plan = folder_upload_plan(prefix, std::path::Path::new(folder)).unwrap_or_else(|error| panic!("{error}"));
    for (path, key) in plan {
        session
            .upload(bucket, &key, &path, &|_| {}, &CancelFlag::new())
            .unwrap_or_else(|error| panic!("{key}: {error}"));
        println!("{key}");
    }
}
