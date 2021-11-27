use proto::yorkie_client::YorkieClient;
use proto::{ActivateClientRequest};

use crate::clientoptions::ClientOptions;

pub mod proto {
    tonic::include_proto!("api");
}

pub struct Client {
    client_id: Option<Vec<u8>>,

    pub rpc_address: String,
    pub options: ClientOptions,
    pub is_active: bool,
}

impl Client {

    pub fn new(rpc_address: String) -> Client {
        Self::with_options(rpc_address, ClientOptions::default())
    }

    pub fn with_options(rpc_address: String, options: ClientOptions) -> Client {
        Self {
            client_id: None,
            rpc_address,
            options,
            is_active: false,
        }
    }

    pub async fn activate(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.is_active {
            return Ok(());
        }

        let mut client = YorkieClient::connect(self.rpc_address.clone()).await?;
        let request = tonic::Request::new(ActivateClientRequest {
            client_key: self.options.key.to_string(),
        });
        let response = client.activate_client(request).await?;
        let message = response.into_inner();
        self.client_id = Some(message.client_id);
        self.is_active = true;

        Ok(())
    }

}