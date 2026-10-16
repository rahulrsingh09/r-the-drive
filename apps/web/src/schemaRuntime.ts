import { z } from 'zod';

// Configure before application schemas so strict CSP never sees an eval probe.
z.config({ jitless: true });
