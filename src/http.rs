use hyper::{
    client::{connect::dns::GaiResolver, HttpConnector},
    Body, Client,
};
use hyper_tls::HttpsConnector;

pub type HttpClient = Client<HttpsConnector<HttpConnector<GaiResolver>>, Body>;

pub async fn body_to_bytes(body: Body) -> anyhow::Result<Vec<u8>> {
    let body = hyper::body::to_bytes(body).await?;
    Ok(body.into())
}

pub fn http_client() -> HttpClient {
    Client::builder().build::<_, Body>(hyper_tls::HttpsConnector::new())
}
