#[cfg(test)]
mod tests {
    use yorkie::{Client, ClientOptions};

    fn init_logger() {
        let _ = env_logger::builder()
            .is_test(true)
            .try_init();
    }

    #[tokio::test]
    async fn client_with_new() {
        init_logger();
        let mut cli = Client::new("http://[::1]:11101".to_string());
        assert_eq!(cli.is_active, false);

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active, true);
        assert_eq!(cli.options.key.len(), 36);
    }

    #[tokio::test]
    async fn client_with_options_test() {
        init_logger();
        let mut cli = Client::with_options("http://[::1]:11101".to_string(), ClientOptions {
            key: "test".to_string(),
            sync_loop_duration: 50,
            reconnect_stream_delay: 1000,
        });
        assert_eq!(cli.options.key, "test");
        assert_eq!(cli.is_active, false);

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active, true);
    }

    #[tokio::test]
    async fn client_deactivate_test() {
        init_logger();
        let mut cli = Client::new("http://[::1]:11101".to_string());
        assert_eq!(cli.is_active, false);

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active, true);
        assert_eq!(cli.options.key.len(), 36);

        let result = cli.deactivate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active, false);
    }
}

