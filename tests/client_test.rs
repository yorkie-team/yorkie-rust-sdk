#[cfg(test)]
mod tests {
    use yorkie::Client;

    #[tokio::test]
    async fn client_test() {
        // 01. Create a new client
        let cli = Client::new("hello".to_string());
        assert_eq!(cli.key, "hello");

        // 02. Activate it
        let result = cli.activate().await;
        assert_eq!(result.is_ok(), true);
    }
}