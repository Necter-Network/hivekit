// HiveKit JavaScript example (runs in the embedded JavaScript engine).
// Build:  npx hivec build examples/counter.js
// Run:    npx hivec run examples/counter.js increment '{"by": 2}'
//
// Handler input is the call input parsed as JSON; return values are
// JSON-encoded (strings are returned as-is).
const { hive, storage, db } = require('hivekit')

hive.define('increment', (input) => {
  const by = input.by === undefined ? 1 : input.by
  if (!Number.isInteger(by) || by <= 0) throw new Error('increment must be a positive integer')
  const count = (db.get('count') || 0) + by
  db.set('count', count)
  hive.emit('incremented', { by, count })
  return { count }
})

hive.define('get', () => ({ count: db.get('count', 0) }))

// relay({address, function, input}): call another module (any language).
hive.define('relay', (input) => {
  const out = hive.call(input.address, input.function, input.input)
  hive.emit('relayed', { function: input.function })
  return { relayed: out }
})

// ctx-style handlers receive a context object instead of (input, ctx).
hive.define('note', (ctx) => {
  storage.set('note', ctx.input.text)
  ctx.log.info('note saved')
  return { saved: true, hash: hive.hash(ctx.input.text) }
})
