//! MCP (Model Context Protocol) server for Cadenza.
//!
//! Provides an MCP server that allows LLMs to interact with the Cadenza language
//! environment, enabling code evaluation, AST inspection, and documentation queries.

use anyhow::Result;
use cadenza_eval::{Compiler, Env, Value};
use cadenza_syntax::{SyntaxNode, parse::parse};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter,
    model::*, schemars, tool, tool_router,
};
use serde::Deserialize;
use std::fmt::Write as _;

/// MCP server for Cadenza language environment
///
/// Note: Each request creates its own Compiler and Env to avoid Send+Sync issues.
#[derive(Clone)]
pub struct CadenzaMcpServer {
    tool_router: ToolRouter<CadenzaMcpServer>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvalRequest {
    /// The Cadenza expression to evaluate
    expression: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParseRequest {
    /// The Cadenza code to parse
    code: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CheckDimensionsRequest {
    /// The Cadenza expression with units to check
    expression: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetDocsRequest {
    /// The symbol name to get documentation for
    symbol: String,
}

#[tool_router]
impl CadenzaMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Evaluate a Cadenza expression and return the result
    #[tool(
        description = "Evaluate a Cadenza expression and return the result. Use this to run calculations, test code, or perform dimensional analysis."
    )]
    async fn eval(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            EvalRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        // Create fresh state for this request
        let mut compiler = Compiler::new();
        let mut env = Env::with_standard_builtins();

        // Parse the expression
        let parsed = parse(&req.expression);
        if !parsed.errors.is_empty() {
            let errors: Vec<String> = parsed
                .errors
                .iter()
                .map(|e| e.message.to_string())
                .collect();
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Parse errors:\n{}",
                errors.join("\n")
            ))]));
        }

        // Evaluate
        let result = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

        // Check for evaluation errors
        if compiler.has_errors() {
            let mut error_msg = String::from("Evaluation errors:\n");
            for diagnostic in compiler.diagnostics() {
                let _ = writeln!(error_msg, "  {}", diagnostic);
            }
            return Ok(CallToolResult::error(vec![Content::text(error_msg)]));
        }

        // Format results using Display trait
        let output = if result.is_empty() {
            "nil".to_string()
        } else if result.len() == 1 {
            format!("{}", result[0])
        } else {
            result
                .iter()
                .enumerate()
                .map(|(i, v)| format!("[{}] {}", i, v))
                .collect::<Vec<_>>()
                .join("\n")
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Parse Cadenza code and display the Abstract Syntax Tree
    #[tool(
        description = "Parse Cadenza code and display the Abstract Syntax Tree (AST). Use this to understand code structure or debug parsing issues."
    )]
    async fn parse(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            ParseRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        let parsed = parse(&req.code);

        // Format diagnostics if any
        let mut output = String::new();
        if !parsed.errors.is_empty() {
            output.push_str("Errors:\n");
            for err in &parsed.errors {
                output.push_str(&format!("  {}\n", err.message));
            }
            output.push('\n');
        }

        // Format AST
        output.push_str("AST:\n");
        output.push_str(&format_syntax_tree(&parsed.syntax(), 0));

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Check dimensional analysis for an expression with units
    #[tool(
        description = "Check dimensional analysis for an expression with units. Use this to verify unit compatibility and conversions."
    )]
    async fn check_dimensions(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            CheckDimensionsRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        let mut compiler = Compiler::new();
        let mut env = Env::with_standard_builtins();

        let parsed = parse(&req.expression);
        if !parsed.errors.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Parse errors encountered".to_string(),
            )]));
        }

        let result = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

        // Check for evaluation errors
        if compiler.has_errors() {
            let mut error_msg = String::from("Evaluation errors:\n");
            for diagnostic in compiler.diagnostics() {
                let _ = writeln!(error_msg, "  {}", diagnostic);
            }
            return Ok(CallToolResult::error(vec![Content::text(error_msg)]));
        }

        // Check the last value
        if let Some(value) = result.last() {
            match value {
                Value::Quantity {
                    value: val,
                    unit,
                    dimension,
                } => Ok(CallToolResult::success(vec![Content::text(format!(
                    "Value: {}\nUnit: {}\nDimension: {}",
                    val, &*unit.name, dimension
                ))])),
                _ => Ok(CallToolResult::success(vec![Content::text(
                    "Expression has no dimensions (dimensionless value)".to_string(),
                )])),
            }
        } else {
            Ok(CallToolResult::success(vec![Content::text(
                "No value produced".to_string(),
            )]))
        }
    }

    /// List all built-in functions and operators
    #[tool(
        description = "List all built-in functions, operators, and special forms available in Cadenza. Use this to discover available functionality."
    )]
    async fn list_builtins(&self) -> Result<CallToolResult, McpError> {
        // Create an environment with standard builtins to query
        let env = Env::with_standard_builtins();

        let mut output = String::new();

        // Iterate through all bindings in the environment
        for (name, value) in env.iter() {
            let type_str = match value {
                Value::SpecialForm(sf) => format!("special-form: {}", sf.name()),
                Value::BuiltinFn(bf) => format!("builtin-fn: {}", bf.name),
                Value::BuiltinMacro(bm) => format!("builtin-macro: {}", bm.name),
                Value::UnitConstructor(unit) => format!("unit: {}", &*unit.name),
                Value::Type(_) => format!("type: {}", name),
                _ => continue, // Skip other values
            };
            let _ = writeln!(output, "{} ({})", name, type_str);
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get documentation for a symbol
    #[tool(
        description = "Get documentation for a specific symbol, function, or operator. Use this to understand how to use language features."
    )]
    async fn get_docs(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            GetDocsRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        // Create an environment with standard builtins to query
        let env = Env::with_standard_builtins();

        // Look up the symbol in the environment
        if let Some(value) = env.get(req.symbol.as_str().into()) {
            let doc = match value {
                Value::SpecialForm(sf) => {
                    format!("{}\n\nSpecial form: {}", req.symbol, sf.name())
                }
                Value::BuiltinFn(bf) => {
                    format!("{}\n\nBuilt-in function: {}", req.symbol, bf.name)
                }
                Value::BuiltinMacro(bm) => {
                    format!("{}\n\nBuilt-in macro: {}", req.symbol, bm.name)
                }
                Value::UnitConstructor(unit) => {
                    format!(
                        "Unit: {}\n\nCreates quantities with this unit.",
                        &*unit.name
                    )
                }
                _ => format!("Symbol found: {}\nValue: {}", req.symbol, value),
            };
            Ok(CallToolResult::success(vec![Content::text(doc)]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No documentation available for symbol: {}",
                req.symbol
            ))]))
        }
    }

    /// Get information about the Cadenza language
    #[tool(
        description = "Get comprehensive information about the Cadenza language, its features, and use cases."
    )]
    async fn about_cadenza(&self) -> Result<CallToolResult, McpError> {
        let about = include_str!("about_cadenza.md");
        Ok(CallToolResult::success(vec![Content::text(about)]))
    }
}

#[rmcp::tool_handler]
impl ServerHandler for CadenzaMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "cadenza-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: Some("Cadenza MCP Server".to_string()),
                website_url: None,
            },
            instructions: Some(
                "MCP server for the Cadenza programming language. Supports expression evaluation, \
                 type inference, dimensional analysis, AST inspection, and documentation queries."
                    .to_string(),
            ),
        }
    }
}

/// Helper function to format syntax tree
fn format_syntax_tree(node: &SyntaxNode, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut result = format!("{}{:?}:\n", indent, node.kind());

    for child in node.children() {
        result.push_str(&format_syntax_tree(&child, depth + 1));
    }

    if node.children().next().is_none() {
        // Leaf node - show text
        let text = node.text();
        if !text.is_empty() {
            result.push_str(&format!("{}  \"{}\"\n", indent, text));
        }
    }

    result
}

/// Start the MCP server with stdio transport
pub async fn start_server() -> Result<()> {
    tracing::info!("Starting Cadenza MCP server");

    let service = CadenzaMcpServer::new()
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| {
            tracing::error!("MCP server error: {:?}", e);
        })?;

    service.waiting().await?;
    Ok(())
}
