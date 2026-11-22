import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Settings2 } from 'lucide-react';

/** Labelled Settings entry for the Car tab. Pass it to the RAppLayout `footer` slot. */
export function RSettingsEntry() {
  return (
    <nav className="r-directory r-settings-entry" aria-label="Car settings">
      <Link to="/settings">
        <Settings2 aria-hidden="true" /><span><strong>Settings</strong><small>Account, sign-in, Rivian account and charging rates</small></span><ArrowRight aria-hidden="true" />
      </Link>
    </nav>
  );
}
