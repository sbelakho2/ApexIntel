// CLI entry point for the deterministic server-UI fixture corpus.
// See e2e/helpers/server-ui-fixtures.cjs for the fixture definitions.
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { seedDatabase } = require('../../e2e/helpers/server-ui-fixtures.cjs');

try {
  await seedDatabase();
  console.log('server-UI fixtures seeded');
} catch (error) {
  console.error('failed to seed server-UI fixtures:', error.message);
  process.exit(1);
}
