"""
HiveKit SDK
===========
Build hive-wasm-v1 modules for NDSR in Python.

    from hivekit import hive

    @hive.define("greet")
    def greet(input):
        return {"message": f"Hello, {input['name']}!"}

``hivec build module.py`` produces a ``.hbc``; ``hivec run module.py greet '{"name":"Ada"}'``
executes it with the NDSR binary when available.
"""

from .version import __version__
from .module import (
    HiveModule,
    HiveCallError,
    HiveAbort,
    _registry,
    _consensus_config,
    _module_config,
    _schedules,
    NRC1Config,
    ConsensusContext,
    ConsensusResult,
    HiveContext,
    HiveDB,
    HiveFiles,
    FileRef,
    FileData,
    HiveLogger,
    NodeInfo,
    RequestInfo,
)
from .canonical import canonical_json, canonical_bytes, keccak256
from .hbc import (
    RUNTIME,
    build_manifest,
    manifest_address,
    normalize_address,
    package_hbc,
    read_hbc,
    validate_manifest,
)
from .compiler import compile_file, compile_source, CompileResult, CompileError

# The global hive object — this is what module code imports.
hive = HiveModule()

__all__ = [
    "__version__",
    "hive",
    "HiveModule",
    "HiveCallError",
    "HiveAbort",
    "NRC1Config",
    "ConsensusContext",
    "ConsensusResult",
    "HiveContext",
    "HiveDB",
    "HiveFiles",
    "FileRef",
    "FileData",
    "HiveLogger",
    "NodeInfo",
    "RequestInfo",
    "canonical_json",
    "canonical_bytes",
    "keccak256",
    "RUNTIME",
    "build_manifest",
    "manifest_address",
    "normalize_address",
    "package_hbc",
    "read_hbc",
    "validate_manifest",
    "compile_file",
    "compile_source",
    "CompileResult",
    "CompileError",
]
