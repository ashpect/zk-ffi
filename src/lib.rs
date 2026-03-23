//! Barretenberg UltraHonk FFI — standalone C-compatible bindings.
//!
//! Provides `bb_prepare`, `bb_prove`, and `bb_verify` for the Barretenberg
//! UltraHonk backend via noir_rs. Completely independent from ProveKit crates.

use {
    anyhow::{bail, Context, Result},
    noir_rs::{
        native_types::{Witness, WitnessMap},
        AcirField, FieldElement,
    },
    std::{
        ffi::CStr,
        os::raw::{c_char, c_int},
        panic,
        path::Path,
    },
};

// Error codes — same values as PKError for consistency at the Swift layer.
const SUCCESS: c_int = 0;
const INVALID_INPUT: c_int = 1;
const SCHEME_READ_ERROR: c_int = 2;
const PROOF_ERROR: c_int = 4;
const FILE_WRITE_ERROR: c_int = 7;

/// Buffer for returning data across FFI. Layout-compatible with PKBuf.
#[repr(C)]
pub struct BBBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl BBBuf {
    fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    fn from_vec(mut v: Vec<u8>) -> Self {
        let ptr = v.as_mut_ptr();
        let len = v.len();
        let cap = v.capacity();
        std::mem::forget(v);
        Self { ptr, len, cap }
    }
}

#[inline]
fn catch_panic<F, T>(default: T, f: F) -> T
where
    F: FnOnce() -> T + panic::UnwindSafe,
{
    panic::catch_unwind(f).unwrap_or(default)
}

fn c_str_to_string(ptr: *const c_char) -> Result<String, c_int> {
    if ptr.is_null() {
        return Err(INVALID_INPUT);
    }
    // Safety: caller guarantees valid null-terminated UTF-8
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(|s| s.to_owned())
        .map_err(|_| INVALID_INPUT)
}

// --- Internal helpers ---

fn read_acir_json(path: &Path) -> Result<(String, serde_json::Value)> {
    let json_str = std::fs::read_to_string(path).context("Failed to read ACIR JSON file")?;
    let v: serde_json::Value =
        serde_json::from_str(&json_str).context("Failed to parse ACIR JSON")?;

    let bytecode = v["bytecode"]
        .as_str()
        .context("ACIR JSON missing 'bytecode' field")?
        .to_owned();
    let abi = v["abi"].clone();

    Ok((bytecode, abi))
}

fn read_inputs(input_path: &Path, abi: &serde_json::Value) -> Result<WitnessMap<FieldElement>> {
    let toml_str = std::fs::read_to_string(input_path).context("Failed to read input TOML")?;
    let toml_table: toml::Table =
        toml::from_str(&toml_str).context("Failed to parse input TOML")?;

    let params = abi["parameters"]
        .as_array()
        .context("ABI missing 'parameters' array")?;

    let mut witness_map = WitnessMap::new();
    let mut idx: u32 = 0;

    for param in params {
        let name = param["name"]
            .as_str()
            .context("ABI parameter missing 'name'")?;
        let typ = &param["type"];
        let toml_val = toml_table
            .get(name)
            .with_context(|| format!("Input TOML missing parameter '{name}'"))?;

        encode_value(toml_val, typ, &mut witness_map, &mut idx)
            .with_context(|| format!("Failed to encode parameter '{name}'"))?;
    }

    Ok(witness_map)
}

fn encode_value(
    toml_val: &toml::Value,
    abi_type: &serde_json::Value,
    map: &mut WitnessMap<FieldElement>,
    idx: &mut u32,
) -> Result<()> {
    let kind = abi_type["kind"]
        .as_str()
        .context("ABI type missing 'kind'")?;

    match kind {
        "field" | "boolean" | "integer" => {
            let fe = toml_to_field_element(toml_val)?;
            map.insert(Witness(*idx), fe);
            *idx += 1;
        }
        "string" => {
            let s = toml_val
                .as_str()
                .context("Expected string for 'string' type")?;
            for byte in s.as_bytes() {
                map.insert(Witness(*idx), FieldElement::from(*byte as u128));
                *idx += 1;
            }
        }
        "array" => {
            let inner_type = &abi_type["type"];
            let arr = toml_val
                .as_array()
                .context("Expected array for 'array' type")?;
            for elem in arr {
                encode_value(elem, inner_type, map, idx)?;
            }
        }
        "tuple" => {
            let fields = abi_type["fields"]
                .as_array()
                .context("Tuple type missing 'fields'")?;
            let arr = toml_val
                .as_array()
                .context("Expected array for tuple type")?;
            for (field_type, val) in fields.iter().zip(arr.iter()) {
                encode_value(val, field_type, map, idx)?;
            }
        }
        "struct" => {
            let fields = abi_type["fields"]
                .as_array()
                .context("Struct type missing 'fields'")?;
            let table = toml_val
                .as_table()
                .context("Expected table for struct type")?;
            for field in fields {
                let field_name = field["name"]
                    .as_str()
                    .context("Struct field missing 'name'")?;
                let field_type = &field["type"];
                let field_val = table
                    .get(field_name)
                    .with_context(|| format!("Struct missing field '{field_name}'"))?;
                encode_value(field_val, field_type, map, idx)?;
            }
        }
        other => bail!("Unsupported ABI type kind: {other}"),
    }
    Ok(())
}

fn toml_to_field_element(val: &toml::Value) -> Result<FieldElement> {
    match val {
        toml::Value::Integer(n) => Ok(FieldElement::from(*n as u128)),
        toml::Value::String(s) => FieldElement::try_from_str(s)
            .ok_or_else(|| anyhow::anyhow!("Invalid field element string: {s}")),
        toml::Value::Boolean(b) => Ok(if *b {
            FieldElement::one()
        } else {
            FieldElement::zero()
        }),
        _ => bail!("Cannot convert TOML value to field element: {val:?}"),
    }
}

// --- FFI functions ---

/// Prepare a circuit for Barretenberg proving/verification.
///
/// Copies the ACIR JSON to `output_dir/circuit.json` and pre-computes the
/// verification key to `output_dir/vk.bin`.
///
/// # Safety
///
/// The caller must ensure that `circuit_path` and `output_dir` are valid
/// null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn bb_prepare(
    circuit_path: *const c_char,
    output_dir: *const c_char,
) -> c_int {
    catch_panic(PROOF_ERROR, || {
        let result = (|| -> Result<(), c_int> {
            let circuit_path = c_str_to_string(circuit_path)?;
            let output_dir_str = c_str_to_string(output_dir)?;

            let output_dir = Path::new(&output_dir_str);
            std::fs::create_dir_all(output_dir).map_err(|_| FILE_WRITE_ERROR)?;

            let (bytecode, _abi) =
                read_acir_json(Path::new(&circuit_path)).map_err(|_| SCHEME_READ_ERROR)?;

            // Copy ACIR JSON so prove() has access to bytecode + ABI.
            std::fs::copy(&circuit_path, output_dir.join("circuit.json"))
                .map_err(|_| FILE_WRITE_ERROR)?;

            // Pre-compute VK.
            noir_rs::barretenberg::srs::setup_srs_from_bytecode(&bytecode, None, false)
                .map_err(|_| PROOF_ERROR)?;

            let vk =
                noir_rs::barretenberg::verify::get_ultra_honk_verification_key(&bytecode, false)
                    .map_err(|_| PROOF_ERROR)?;

            std::fs::write(output_dir.join("vk.bin"), &vk).map_err(|_| FILE_WRITE_ERROR)?;

            Ok(())
        })();

        match result {
            Ok(()) => SUCCESS,
            Err(code) => code,
        }
    })
}

/// Prove using the Barretenberg UltraHonk backend.
///
/// # Safety
///
/// The caller must ensure that:
/// - `scheme_dir` and `input_path` are valid null-terminated C strings
/// - `out_buf` is a valid pointer to a `BBBuf` structure
/// - The returned buffer is freed using `bb_free_buf`
#[no_mangle]
pub unsafe extern "C" fn bb_prove(
    scheme_dir: *const c_char,
    input_path: *const c_char,
    out_buf: *mut BBBuf,
) -> c_int {
    if out_buf.is_null() {
        return INVALID_INPUT;
    }

    catch_panic(PROOF_ERROR, || {
        let out_buf = &mut *out_buf;
        *out_buf = BBBuf::empty();

        let result = (|| -> Result<Vec<u8>, c_int> {
            let scheme_dir_str = c_str_to_string(scheme_dir)?;
            let input_path = c_str_to_string(input_path)?;

            let scheme_dir = Path::new(&scheme_dir_str);

            let (bytecode, abi) =
                read_acir_json(&scheme_dir.join("circuit.json")).map_err(|_| SCHEME_READ_ERROR)?;
            let initial_witness =
                read_inputs(Path::new(&input_path), &abi).map_err(|_| INVALID_INPUT)?;

            let vk = std::fs::read(scheme_dir.join("vk.bin")).map_err(|_| SCHEME_READ_ERROR)?;

            noir_rs::barretenberg::srs::setup_srs_from_bytecode(&bytecode, None, false)
                .map_err(|_| PROOF_ERROR)?;

            let proof = noir_rs::barretenberg::prove::prove_ultra_honk(
                &bytecode,
                initial_witness,
                vk,
                false,
            )
            .map_err(|_| PROOF_ERROR)?;

            Ok(proof)
        })();

        match result {
            Ok(proof_bytes) => {
                *out_buf = BBBuf::from_vec(proof_bytes);
                SUCCESS
            }
            Err(code) => code,
        }
    })
}

/// Verify a proof using the Barretenberg UltraHonk backend.
///
/// # Safety
///
/// The caller must ensure that:
/// - `proof_ptr` points to `proof_len` valid bytes
/// - `scheme_dir` is a valid null-terminated C string
#[no_mangle]
pub unsafe extern "C" fn bb_verify(
    proof_ptr: *const u8,
    proof_len: usize,
    scheme_dir: *const c_char,
) -> c_int {
    catch_panic(PROOF_ERROR, || {
        let result = (|| -> Result<bool, c_int> {
            if proof_ptr.is_null() || proof_len == 0 {
                return Err(INVALID_INPUT);
            }

            let scheme_dir_str = c_str_to_string(scheme_dir)?;
            let proof_bytes = std::slice::from_raw_parts(proof_ptr, proof_len);

            let vk = std::fs::read(Path::new(&scheme_dir_str).join("vk.bin"))
                .map_err(|_| SCHEME_READ_ERROR)?;

            let valid =
                noir_rs::barretenberg::verify::verify_ultra_honk(proof_bytes.to_vec(), vk)
                    .map_err(|_| PROOF_ERROR)?;

            Ok(valid)
        })();

        match result {
            Ok(true) => SUCCESS,
            Ok(false) => PROOF_ERROR,
            Err(code) => code,
        }
    })
}

/// Free a buffer allocated by Barretenberg FFI functions.
///
/// # Safety
///
/// The caller must ensure that:
/// - The buffer was allocated by a Barretenberg FFI function
/// - The buffer is not used after calling this function
/// - This function is called exactly once for each allocated buffer
#[no_mangle]
pub unsafe extern "C" fn bb_free_buf(buf: BBBuf) {
    if !buf.ptr.is_null() && buf.cap > 0 {
        drop(Vec::from_raw_parts(buf.ptr, buf.len, buf.cap));
    }
}
