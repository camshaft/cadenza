//! MCP (Model Context Protocol) server for Cadenza.
//!
//! Provides an MCP server that allows LLMs to interact with the Cadenza language
//! environment, enabling code evaluation, type checking, dimensional analysis,
//! AST inspection, and documentation queries.

use anyhow::Result;
use cadenza_eval::{Compiler, Env, Value};
use cadenza_syntax::{parse::parse, SyntaxNode};
use rmcp::{
    handler::server::router::tool::ToolRouter, model::*, schemars, tool, tool_router,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::Deserialize;

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
struct InferTypeRequest {
    /// The Cadenza expression to infer type for
    expression: String,
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
    #[tool(description = "Evaluate a Cadenza expression and return the result. Use this to run calculations, test code, or perform dimensional analysis.")]
    async fn eval(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            EvalRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        // Create fresh state for this request
        let mut compiler = Compiler::new();
        let mut env = Env::new();

        // Parse the expression
        let parsed = parse(&req.expression);
        if !parsed.errors.is_empty() {
            let errors: Vec<String> = parsed
                .errors
                .iter()
                .map(|e| format!("{}", e.message))
                .collect();
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Parse errors:\n{}",
                errors.join("\n")
            ))]));
        }

        // Evaluate
        let result = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);
        // eval returns Vec<Value>, one for each top-level expression
        let formatted_results: Vec<String> = result.iter().map(|v| format_value(v)).collect();
        let output = if formatted_results.is_empty() {
            "nil".to_string()
        } else {
            formatted_results.join("\n")
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Parse Cadenza code and display the Abstract Syntax Tree
    #[tool(description = "Parse Cadenza code and display the Abstract Syntax Tree (AST). Use this to understand code structure or debug parsing issues.")]
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
            output.push_str("\n");
        }

        // Format AST
        output.push_str("AST:\n");
        output.push_str(&format_syntax_tree(&parsed.syntax(), 0));

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Infer the type of a Cadenza expression
    #[tool(description = "Infer the type of a Cadenza expression. Use this to understand what type an expression evaluates to.")]
    async fn infer_type(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            InferTypeRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        let mut compiler = Compiler::new();
        let mut env = Env::new();

        let parsed = parse(&req.expression);
        if !parsed.errors.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Parse errors encountered".to_string(),
            )]));
        }

        // Evaluate to get the type
        let result = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);
        // eval returns Vec<Value>, we'll check the type of the last value
        let type_str = if let Some(value) = result.last() {
            match value {
                Value::Nil => "Nil".to_string(),
                Value::Bool(_) => "Bool".to_string(),
                Value::Symbol(_) => "Symbol".to_string(),
                Value::Integer(_) => "Integer".to_string(),
                Value::Float(_) => "Float".to_string(),
                Value::String(_) => "String".to_string(),
                Value::List(_) => "List".to_string(),
                Value::Record { .. } => "Record".to_string(),
                Value::StructConstructor { name, .. } => format!("StructConstructor<{}>", name),
                Value::Type(_) => "Type".to_string(),
                Value::Quantity { dimension, .. } => format!("Quantity<{}>", dimension),
                Value::UnitConstructor(_) => "UnitConstructor".to_string(),
                Value::BuiltinFn(_) => "BuiltinFn".to_string(),
                Value::BuiltinMacro(_) => "BuiltinMacro".to_string(),
                Value::SpecialForm(_) => "SpecialForm".to_string(),
                Value::UserFunction(_) => "UserFunction".to_string(),
            }
        } else {
            "Nil".to_string()
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Type: {}",
            type_str
        ))]))
    }

    /// Check dimensional analysis for an expression with units
    #[tool(description = "Check dimensional analysis for an expression with units. Use this to verify unit compatibility and conversions.")]
    async fn check_dimensions(
        &self,
        rmcp::handler::server::wrapper::Parameters(
            req,
        ): rmcp::handler::server::wrapper::Parameters<CheckDimensionsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut compiler = Compiler::new();
        let mut env = Env::new();

        let parsed = parse(&req.expression);
        if !parsed.errors.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Parse errors encountered".to_string(),
            )]));
        }

        let result = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);
        // Check the last value
        if let Some(value) = result.last() {
            match value {
                Value::Quantity {
                    value: val,
                    unit,
                    dimension,
                } => Ok(CallToolResult::success(vec![Content::text(format!(
                    "Value: {}\nDimension: {}\nUnit: {:?}",
                    val, dimension, unit
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
    #[tool(description = "List all built-in functions, operators, and special forms available in Cadenza. Use this to discover available functionality.")]
    async fn list_builtins(&self) -> Result<CallToolResult, McpError> {
        let builtins = vec![
            "Operators: +, -, *, /, ==, !=, <, <=, >, >=, |>",
            "Special Forms: let, fn, match, assert, typeof, measure",
            "Math Functions: (to be implemented)",
            "List Functions: (to be implemented)",
            "String Functions: (to be implemented)",
        ];

        Ok(CallToolResult::success(vec![Content::text(
            builtins.join("\n"),
        )]))
    }

    /// Get documentation for a symbol
    #[tool(description = "Get documentation for a specific symbol, function, or operator. Use this to understand how to use language features.")]
    async fn get_docs(
        &self,
        rmcp::handler::server::wrapper::Parameters(req): rmcp::handler::server::wrapper::Parameters<
            GetDocsRequest,
        >,
    ) -> Result<CallToolResult, McpError> {
        let docs = match req.symbol.as_str() {
            "let" => "let name = expression\n\nBind a value to a name in the current scope.",
            "fn" => "fn name param1 param2 ... = body\n\nDefine a function with parameters.",
            "measure" => {
                "measure name\nmeasure name = base_unit factor\n\nDefine a unit of measurement."
            }
            "+" => "a + b\n\nAddition operator. Works with numbers and compatible units.",
            "-" => "a - b\n\nSubtraction operator. Works with numbers and compatible units.",
            "*" => "a * b\n\nMultiplication operator. Can multiply values with units.",
            "/" => "a / b\n\nDivision operator. Can divide values with units.",
            "|>" => "a |> f\n\nPipeline operator. Passes the left value as argument to the right function.",
            _ => "No documentation available for this symbol.",
        };

        Ok(CallToolResult::success(vec![Content::text(docs)]))
    }

    /// Get information about the Cadenza language
    #[tool(description = "Get comprehensive information about the Cadenza language, its features, and use cases.")]
    async fn about_cadenza(&self) -> Result<CallToolResult, McpError> {
        let about = r#"# Cadenza Programming Language

Cadenza is a functional programming language with first-class support for units of measure and dimensional analysis.

## Key Features

1. **Units of Measure**: Built-in support for physical units and automatic dimensional analysis
   - Define base units: `measure meter`
   - Define derived units: `measure kilometer = meter 1000`
   - Use in expressions: `10meter`, `5kilometer`
   - Automatic unit checking: prevents adding incompatible units

2. **Functional Programming**: Functions are first-class values
   - Define functions: `fn square x = x * x`
   - Anonymous functions: `fn x -> x + 1`
   - Higher-order functions supported

3. **Type Safety**: Static type checking with type inference
   - Types are inferred automatically
   - Compile-time error detection
   - No runtime type errors

4. **Dimensional Analysis**: Compile-time verification of physical dimensions
   - Prevents dimension mismatches: `10meter + 5second` is an error
   - Tracks derived dimensions: `distance / time` gives velocity
   - Unit conversions are explicit

5. **Interactive REPL**: Immediate feedback and exploration
   - Evaluate expressions interactively
   - Define and test functions
   - Load and save scripts

## Primary Use Cases

- **Quick Calculations**: Fast computation with proper unit handling
- **3D Modeling**: Define models in code (like OpenSCAD)
- **Algorithmic Music**: Compose music with code
- **Interactive Books**: Drive simulations and visualizations
- **Scientific Computing**: Calculations with dimensional analysis

## Example Code

```cadenza
# Define units
measure meter
measure second

# Use units in calculations
let distance = 100meter
let time = 10second
let speed = distance / time  # Automatically: meter/second

# Define functions
fn kinetic_energy mass velocity =
    0.5 * mass * velocity * velocity

# Use functions
let energy = kinetic_energy 1000kilogram 20meter/second
```

## Syntax Basics

- Variables: `let name = value`
- Functions: `fn name param = body`
- Comments: `# This is a comment`
- Pipeline: `value |> function`
- Operators: `+`, `-`, `*`, `/`, `==`, `!=`, `<`, `<=`, `>`, `>=`

For more examples, use the `eval` tool to try expressions!
"#;

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

/// Helper function to format values for display
fn format_value(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Symbol(s) => s.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => format!("\"{}\"", s),
        Value::List(items) => {
            let formatted: Vec<String> = items.iter().map(|v| format_value(v)).collect();
            format!("[{}]", formatted.join(", "))
        }
        Value::Record { type_name, fields } => {
            let fields_str: Vec<String> = fields
                .iter()
                .map(|(name, val)| format!("{} = {}", name, format_value(val)))
                .collect();
            match type_name {
                Some(name) => format!("{} {{ {} }}", name, fields_str.join(", ")),
                None => format!("{{ {} }}", fields_str.join(", ")),
            }
        }
        Value::StructConstructor { name, .. } => format!("<struct-constructor {}>", name),
        Value::Type(_) => "<type>".to_string(),
        Value::Quantity {
            value,
            unit,
            dimension: _,
        } => format!("{}{:?}", value, unit),
        Value::UnitConstructor(unit) => format!("<unit {:?}>", unit),
        Value::BuiltinFn(f) => format!("<builtin-fn {}>", f.name),
        Value::BuiltinMacro(m) => format!("<builtin-macro {}>", m.name),
        Value::SpecialForm(sf) => format!("<special-form {}>", sf.name),
        Value::UserFunction(_) => "<user-function>".to_string(),
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
