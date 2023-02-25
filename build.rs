extern crate prost_build;

use std::env;

fn main() {
    let current_dir = env::current_dir()
        .unwrap()
        .into_os_string()
        .into_string()
        .unwrap();
    env::set_var("OUT_DIR", current_dir + "/src");
    prost_build::compile_protos(&["src/message.proto"], &["src/"]).unwrap();
}
