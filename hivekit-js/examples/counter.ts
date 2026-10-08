// HiveKit AssemblyScript example (default target for .ts).
// Build:  npx hivec build examples/counter.ts
// Run:    npx hivec run examples/counter.ts increment '5'
//
// Handlers take the input string and return the output string. `hive`,
// `storage` are provided by the HiveKit prelude; the import is for editors only.
import { hive, storage } from 'hivekit'

function readCount(): i64 {
  const cur = storage.get("count");
  return cur.length == 0 ? 0 : I64.parseInt(cur);
}

// increment(by): add `by` (default 1), persist, emit an event, return the new count.
function increment(input: string): string {
  const by: i64 = input.length == 0 ? 1 : I64.parseInt(input);
  if (by <= 0) {
    hive.fail("increment must be positive, got " + input);
  }
  const next = readCount() + by;
  storage.set("count", next.toString());
  hive.emit("incremented", "{\"by\":" + by.toString() + ",\"count\":" + next.toString() + "}");
  return next.toString();
}

function get(input: string): string {
  return readCount().toString();
}

// relay("<address>|<function>|<input>"): call another module and return its output.
function relay(input: string): string {
  const parts = input.split("|");
  if (parts.length != 3) {
    hive.fail("relay input must be address|function|input");
  }
  const out = hive.call(parts[0], parts[1], parts[2]);
  hive.emit("relayed", "{\"function\":\"" + parts[1] + "\"}");
  return out;
}

function fingerprint(input: string): string {
  return hive.hash(input);
}

hive.define("increment", increment);
hive.define("get", get);
hive.define("relay", relay);
hive.define("fingerprint", fingerprint);
