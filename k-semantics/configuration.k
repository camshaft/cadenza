// Cadenza Configuration Module
//
// This module defines the runtime configuration (state) for executing
// Cadenza programs. The configuration is organized into cells that hold
// different aspects of program state.

requires "syntax.k"

module CADENZA-CONFIGURATION
  imports CADENZA-SYNTAX
  imports MAP
  imports LIST

  // =========================================================================
  // Runtime Configuration
  // =========================================================================
  
  configuration
    <cadenza>
      // The computation cell holds the current program/expression being evaluated
      <k> $PGM:Pgm </k>
      
      // Environment: maps identifiers to values
      // Organized as a stack of scopes for lexical scoping
      <env> .List </env>
      
      // Current scope: map from Id to Value
      <scope> .Map </scope>
      
      // Return value cell (for function returns)
      <return> .K </return>
      
      // Output for printing/debugging
      <output stream="stdout"> .List </output>
      
      // Input for reading (future use)
      <input stream="stdin"> .List </input>
      
      // Error/exception cell
      <error> .K </error>
    </cadenza>

endmodule
