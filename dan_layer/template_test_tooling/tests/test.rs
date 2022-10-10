//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[cfg(test)]
mod test {
    use tari_dan_engine::{
        packager::{PackageError, TemplateModuleLoader},
        state_store::{AtomicDb, StateReader},
        wasm::{compile::compile_template, WasmExecutionError},
    };
    use tari_engine_types::instruction::Instruction;
    use tari_template_lib::{
        args,
        models::{Amount, ComponentAddress, ComponentInstance},
    };
    use tari_template_test_tooling::TemplateTest;

    #[test]
    fn test_hello_world() {
        let template_test = TemplateTest::new(vec!["tests/templates/hello_world"]);
        let result: String = template_test.call_function("HelloWorld", "greet", args![]);

        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_state() {
        let template_test = TemplateTest::new(vec!["tests/templates/state"]);
        let store = template_test.state_store();

        // constructor
        let component_address1: ComponentAddress = template_test.call_function("State", "new", args![]);
        template_test.assert_calls(&[
            "set_current_runtime_state",
            "emit_log",
            "create_component",
            "set_last_instruction_output",
            "finalize",
        ]);
        template_test.clear_calls();

        let component_address2: ComponentAddress = template_test.call_function("State", "new", args![]);
        assert_ne!(component_address1, component_address2);

        let component: ComponentInstance = store
            .read_access()
            .unwrap()
            .get_state(&component_address1)
            .unwrap()
            .expect("component1 not found");
        assert_eq!(component.module_name, "State");

        let component: ComponentInstance = store
            .read_access()
            .unwrap()
            .get_state(&component_address2)
            .unwrap()
            .expect("component2 not found");
        assert_eq!(component.module_name, "State");

        // call the "set" method to update the instance value
        let new_value = 20_u32;
        template_test.call_method::<()>(component_address2, "set", args![new_value]);

        // call the "get" method to get the current value
        let value: u32 = template_test.call_method(component_address2, "get", args![]);

        assert_eq!(value, new_value);
    }

    #[test]
    fn test_composed() {
        let template_test = TemplateTest::new(vec!["tests/templates/state", "tests/templates/hello_world"]);

        let functions = template_test
            .get_module("HelloWorld")
            .template_def()
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(functions, vec!["greet", "new", "custom_greeting"]);

        let functions = template_test
            .get_module("State")
            .template_def()
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(functions, vec!["new", "set", "get"]);

        let component_state: ComponentAddress = template_test.call_function("State", "new", args![]);
        let component_hw: ComponentAddress = template_test.call_function("HelloWorld", "new", args!["أهلا"]);

        let result: String = template_test.call_method(component_hw, "custom_greeting", args!["Wasm"]);
        assert_eq!(result, "أهلا Wasm!");

        // call the "set" method to update the instance value
        let new_value = 20_u32;
        template_test.call_method::<()>(component_state, "set", args![new_value]);

        // call the "get" method to get the current value
        let value: u32 = template_test.call_method(component_state, "get", args![]);

        assert_eq!(value, new_value);
    }

    #[test]
    fn test_dodgy_template() {
        let err = compile_template("tests/templates/buggy", &["call_engine_in_abi"])
            .unwrap()
            .load_template()
            .unwrap_err();
        assert!(matches!(err, PackageError::TemplateCalledEngineDuringInitialization));

        let err = compile_template("tests/templates/buggy", &["return_null_abi"])
            .unwrap()
            .load_template()
            .unwrap_err();
        assert!(matches!(
            err,
            PackageError::WasmModuleError(WasmExecutionError::AbiDecodeError)
        ));

        let err = compile_template("tests/templates/buggy", &["unexpected_export_function"])
            .unwrap()
            .load_template()
            .unwrap_err();
        assert!(matches!(
            err,
            PackageError::WasmModuleError(WasmExecutionError::UnexpectedAbiFunction { .. })
        ));
    }

    #[test]
    fn test_erc20() {
        let template_test = TemplateTest::new(vec!["tests/templates/erc20"]);

        let initial_supply = Amount(1_000_000_000_000);
        let owner_address: ComponentAddress =
            template_test.call_function("FungibleAccount", "initial_mint", args![initial_supply]);

        let receiver_address: ComponentAddress = template_test.call_method(owner_address, "new_account", args![]);

        let result = template_test.execute(vec![
            Instruction::CallMethod {
                template_address: template_test.get_template_address("FungibleAccount"),
                component_address: owner_address,
                method: "withdraw".to_string(),
                args: args![Amount(100)],
            },
            Instruction::PutLastInstructionOutputOnWorkspace {
                key: b"foo_bucket".to_vec(),
            },
            Instruction::CallMethod {
                template_address: template_test.get_template_address("FungibleAccount"),
                component_address: receiver_address,
                method: "deposit".to_string(),
                args: args![Workspace(b"foo_bucket")],
            },
            Instruction::CallMethod {
                template_address: template_test.get_template_address("FungibleAccount"),
                component_address: owner_address,
                method: "balance".to_string(),
                args: args![],
            },
            Instruction::CallMethod {
                template_address: template_test.get_template_address("FungibleAccount"),
                component_address: receiver_address,
                method: "balance".to_string(),
                args: args![],
            },
        ]);
        for log in result.logs {
            eprintln!("LOG: {}", log);
        }
        eprintln!("{:?}", result.execution_results);
        assert_eq!(
            result.execution_results[3].decode::<Amount>().unwrap(),
            initial_supply - 100
        );
        assert_eq!(result.execution_results[4].decode::<Amount>().unwrap(), 100);
    }

    #[test]
    fn test_private_function() {
        // instantiate the counter
        let template_test = TemplateTest::new(vec!["tests/templates/private_function"]);

        // check that the private method and function are not exported
        let functions = template_test
            .get_module("PrivateCounter")
            .template_def()
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(functions, vec!["new", "get", "increase"]);

        // check that public methods can still internally call private ones
        let component: ComponentAddress = template_test.call_function("PrivateCounter", "new", args![]);
        template_test.call_method::<()>(component, "increase", args![]);
        let value: u32 = template_test.call_method(component, "get", args![]);
        assert_eq!(value, 1);
    }

    #[test]
    fn test_tuples() {
        let template_test = TemplateTest::new(vec!["tests/templates/tuples"]);

        // tuples returned in a regular function
        let (message, number): (String, u32) = template_test.call_function("Tuple", "tuple_output", args![]);
        assert_eq!(message, "Hello World!");
        assert_eq!(number, 100);

        // tuples returned in a constructor
        template_test.clear_calls();
        let (component_id, message): (ComponentAddress, String) = template_test.call_function("Tuple", "new", args![]);
        assert_eq!(message, "Hello World!");

        // the component id returned in the tuple must be valid and usable
        let new_value = 20_u32;
        template_test.call_method::<()>(component_id, "set", args![new_value]);
        let value: u32 = template_test.call_method(component_id, "get", args![]);
        assert_eq!(value, new_value);
    }
}
