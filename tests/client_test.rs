#[cfg(test)]
mod tests {
    use yorkie::Client;

    #[tokio::test]
    async fn client_test() {
        let cli = Client::new("hello".to_string());
        assert_eq!(cli.key, "hello");

        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
    }
}