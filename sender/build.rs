fn main() {
    slint_build::compile("ui/main.slint").unwrap();
    // TODO: accelerated preview rendering for windows
    cfg_aliases::cfg_aliases! {
       egl_preview: { any(target_os = "linux", target_os = "windows") },
    }
}
