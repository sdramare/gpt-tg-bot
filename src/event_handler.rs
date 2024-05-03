use lambda_http::Request;
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait EventHandler {
    async fn process_event(&self, event: &Request) -> anyhow::Result<()>;
}
