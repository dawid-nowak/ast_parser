# ast_parser

Usage :

Will create common types and create APIs using those. Additionally will output the mapped types to "./mapped_types_to_names.txt" and "./mapped_names.txt" files. ./mapped_names.txt can be use as a source to provide some customer/more sensible substitutes for type names in step two.

1. Step 1
```bash
cargo run -- --apis-dir ../gateway-api-rs/gateway-api/src/apis/standard --out-dir ../gateway-api-rs/gateway-api/src/apis/processed 
```

2. Step 2 (Optional)
Create a file with new type names. The file format is like this:
```
# type_name->new_type_name where type_name is taken from ./mapped_names.txt

CommonAddressesGateway->AddressGateway
CommonBackendFiltersMirrorRefRequestRouteRules->MirrorBackendRef
CommonExtensionFiltersRefRouteRules->FilterExtensionRef
CommonFiltersHeaderModifierRouteRules->HeaderModifier
CommonGatewayKindsListeners->ListenerRouteKinds
CommonParentRoute->ParentRef

```
3. Step 3 (Optional)

Will read a file specified by --with-substitute-names and try to use those names as substitutions when modifying the APIs.

```bash
cargo run -- --apis-dir ../gateway-api-rs/gateway-api/src/apis/standard --out-dir ../gateway-api-rs/gateway-api/src/apis/processed --with-substitute-names ./custom_mapped_names.txt
```

