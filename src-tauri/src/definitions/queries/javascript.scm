(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(class_declaration name: (identifier) @name) @definition.class
(method_definition name: (property_identifier) @name) @definition.method
(field_definition property: (property_identifier) @name) @definition.field

; Top-level and exported bindings (not function-local ones).
(program (lexical_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(program (variable_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(export_statement (lexical_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(export_statement (variable_declaration (variable_declarator name: (identifier) @name)) @definition.variable)

; { name: () => ..., name: function () {} }
(pair key: (property_identifier) @name value: [(arrow_function) (function_expression)]) @definition.function
