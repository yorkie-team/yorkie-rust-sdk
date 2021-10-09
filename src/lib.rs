use proto::yorkie_client::YorkieClient;
use proto::ActivateClientRequest;

pub mod proto {
    tonic::include_proto!("api");
}

pub struct Client {
    pub key: String,
}

impl Client {
    pub fn new(key: String) -> Self {
        Self {
            key: key,
        }
    }

    pub async fn activate(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut client = YorkieClient::connect("http://[::1]:11101").await?;
        let request = tonic::Request::new(ActivateClientRequest {
            client_key: self.key.clone(),
        });

        let response = client.activate_client(request).await?;
        println!("{:?}", response);

        Ok(())
    }
}