use yorkie::yorkie_client::YorkieClient;
use yorkie::ActivateClientRequest;

pub mod yorkie {
    tonic::include_proto!("api");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = YorkieClient::connect("http://[::1]:11101").await?;

    let request = tonic::Request::new(ActivateClientRequest {
        client_key: "HelloRustSDK".into(),
    });

    let response = client.activate_client(request).await?;

    println!("RESPONSE={:?}", response);

    Ok(())
}
