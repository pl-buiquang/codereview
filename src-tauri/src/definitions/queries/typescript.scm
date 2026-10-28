(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(function_signature name: (identifier) @name) @definition.function
(class_declaration name: (type_identifier) @name) @definition.class
(abstract_class_declaration name: (type_identifier) @name) @definition.class
(interface_declaration name: (type_identifier) @name) @definition.interface
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
(method_definition name: (property_identifier) @name) @definition.method
(method_signature name: (property_identifier) @name) @definition.method
(abstract_method_signature name: (property_identifier) @name) @definition.method
(public_field_definition name: (property_identifier) @name) @definition.field
(property_signature name: (property_identifier) @name) @definition.field

; Top-level and exported bindings (not function-local ones).
(program (lexical_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(program (variable_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(export_statement (lexical_declaration (variable_declarator name: (identifier) @name)) @definition.variable)
(export_statement (variable_declaration (variable_declarator name: (identifier) @name)) @definition.variable)

(pair key: (property_identifier) @name value: [(arrow_function) (function_expression)]) @definition.function
