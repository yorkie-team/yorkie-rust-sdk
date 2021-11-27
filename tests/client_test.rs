#[cfg(test)]
mod tests {
    use yorkie::{Client, clientoptions::ClientOptions};


    #[tokio::test]
    async fn client_with_new() {
        let cli = Client::new("http://[::1]:11101".to_string());
        assert_eq!(cli.is_active.get(), false);

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active.get(), true);
        assert_eq!(cli.options.key.len(), 36);
        assert_eq!(cli.is_active.get(), false);
    }

    #[tokio::test]
    async fn client_with_options_test() {
        let cli = Client::with_options("http://[::1]:11101".to_string(), ClientOptions {
            key: "test".to_string(),
            sync_loop_duration: 50,
            reconnect_stream_delay: 1000,
        });
        assert_eq!(cli.options.key, "test");
        assert_eq!(cli.is_active.get(), false);

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
        assert_eq!(cli.is_active.get(), true);
    }
}