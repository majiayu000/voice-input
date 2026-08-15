pub(crate) fn install_default_provider() {
    #[cfg(target_os = "macos")]
    {
        // Installation is process-global. A later call returns the already
        // installed provider, so this remains safe across network adapters.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
