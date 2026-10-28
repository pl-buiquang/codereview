(function_definition name: (identifier) @name) @definition.function
(class_definition name: (identifier) @name) @definition.class
; Module-level names (constants, aliases).
(module (expression_statement (assignment left: (identifier) @name)) @definition.variable)
