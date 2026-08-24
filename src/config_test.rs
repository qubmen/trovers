#[cfg(test)]
mod tests {
    use crate::config::Config;

    /// A config.toml written before video playback existed. `Config::load` reads a
    /// fixed path under the user's home, so the parse is tested where the decision
    /// actually lives: serde.
    const LEGACY: &str = r#"
default_speed = 1.0
default_volume = 80
"#;

    #[test]
    fn a_config_without_video_args_loads_with_none() {
        let config: Config = toml::from_str(LEGACY).expect("legacy config parses");
        assert!(
            config.video_mpv_args.is_empty(),
            "an absent key must not invent flags: mpv exits on one it does not know"
        );
    }

    #[test]
    fn the_default_config_carries_no_video_args() {
        assert!(Config::default().video_mpv_args.is_empty());
    }

    #[test]
    fn configured_video_args_round_trip() {
        let config = Config {
            video_mpv_args: vec!["--focus-on=never".to_string(), "--ontop".to_string()],
            ..Config::default()
        };
        let raw = toml::to_string(&config).expect("serializes");
        let back: Config = toml::from_str(&raw).expect("parses back");
        assert_eq!(
            back.video_mpv_args,
            vec!["--focus-on=never".to_string(), "--ontop".to_string()]
        );
    }

    #[test]
    fn a_missing_config_is_created_with_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let config = Config::load_from(&path).expect("writes and returns defaults");

        assert_eq!(config.default_speed, Config::default().default_speed);
        assert!(
            path.exists(),
            "a default config must be written so the next launch has something to edit"
        );
    }

    #[test]
    fn a_corrupt_config_is_backed_up_and_replaced_with_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_speed = [not valid toml").expect("write broken config");

        let config = Config::load_from(&path).expect("falls back instead of erroring out");

        assert_eq!(config.default_speed, Config::default().default_speed);
        let backup = path.with_extension("toml.bad");
        assert!(
            backup.exists(),
            "the broken file must be preserved for inspection, not lost"
        );
    }

    #[test]
    fn an_out_of_range_speed_and_volume_are_clamped_on_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_speed = 100.0\ndefault_volume = 250\n")
            .expect("write config");

        let config = Config::load_from(&path).expect("loads");

        assert_eq!(config.default_speed, 3.0, "speed must clamp at mpv's max");
        assert_eq!(config.default_volume, 100, "volume must clamp at 100");
    }
}
