import { _api } from '@iconify/react';

// Missing remote icons must not reveal browser traffic to a CDN.
_api.setFetch(async () => new Response(null, { status: 404 }));
