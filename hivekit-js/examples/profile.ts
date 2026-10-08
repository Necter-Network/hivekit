// Full TypeScript in the JavaScript engine: build with `--target js`.
//   npx hivec build examples/profile.ts --target js
import { hive, db } from 'hivekit'

interface Profile {
  name: string
  tags: string[]
}

class Profiles {
  static key(id: string): string {
    return `profile:${id}`
  }
  static save(id: string, p: Profile): Profile {
    db.set(Profiles.key(id), p)
    return p
  }
}

hive.define('saveProfile', async (input: { id: string; profile: Profile }) => {
  const saved = Profiles.save(input.id, { ...input.profile, tags: [...new Set(input.profile.tags)].sort() })
  hive.emit('profile_saved', { id: input.id, tags: saved.tags.length })
  return saved
})

hive.define('getProfile', (input: { id: string }) => db.get<Profile>(Profiles.key(input.id)) ?? hive.fail(`no profile ${input.id}`))
