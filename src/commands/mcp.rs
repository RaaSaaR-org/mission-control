use crate::config::ResolvedConfig;
use crate::error::McResult;
use crate::mcp::McServer;
use rmcp::ServiceExt;

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    // Send tracing output to stderr so it doesn't corrupt the MCP stdio channel.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing_subscriber::filter::LevelFilter::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let server = McServer::new(cfg.clone());
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| crate::error::McError::Other(format!("MCP server error: {}", e)))?;
        service
            .waiting()
            .await
            .map_err(|e| crate::error::McError::Other(format!("MCP server error: {}", e)))?;
        Ok(())
    })
}
