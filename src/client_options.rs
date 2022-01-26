use uuid::Uuid;

#[derive(Clone, Default)]
pub struct ClientOptions {
    pub key: String,
    pub sync_loop_duration: u32,
    pub reconnect_stream_delay: u32,
}

impl ClientOptions {
    pub fn default() -> ClientOptions {
        let key = Uuid::new_v4().to_hyphenated().to_string();

        ClientOptions {
            key,
            sync_loop_duration: 50,
            reconnect_stream_delay: 1000,
        }
    }
}
