use std::cell::Cell;

use proto::yorkie_client::YorkieClient;
use proto::ActivateClientRequest;

use crate::clientoptions::ClientOptions;

pub mod proto {
    tonic::include_proto!("api");
}

pub struct Client {
    pub rpc_address: String,
    pub options: ClientOptions,
    pub is_active: Cell<bool>,
}

impl Client {

    pub fn new(rpc_address: String) -> Client {
        Self::with_options(rpc_address, ClientOptions::default())
    }

    pub fn with_options(rpc_address: String, options: ClientOptions) -> Client {
        Self {
            rpc_address,
            options,
            is_active: Cell::new(false),
        }
    }

    pub async fn activate(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut client = YorkieClient::connect(self.rpc_address.clone()).await?;
        let request = tonic::Request::new(ActivateClientRequest {
            client_key: self.options.key.to_string(),
        });

        let response = client.activate_client(request).await?;
        println!("{:?}", response);

        self.is_active.set(true);
        Ok(())
    }
}