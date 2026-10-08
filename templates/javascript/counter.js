const { hive, db } = require('hivekit')

hive.define('increment', (input) => {
  const by = input.by === undefined ? 1 : input.by
  if (!Number.isInteger(by) || by <= 0) throw new Error('by must be a positive integer')
  const count = (db.get('count') || 0) + by
  db.set('count', count)
  hive.emit('incremented', { by, count })
  return { count }
})

hive.define('get', () => ({ count: db.get('count', 0) }))
