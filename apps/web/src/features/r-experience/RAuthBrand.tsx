import React from 'react';
import { RMark } from './RMark';

/** Logo and tagline above the sign-in and activation forms. */
export function RAuthBrand() {
  return <div className="r-auth-brand"><RMark size={72} className="r-auth-logo" /><p>Your Rivian, understood.</p></div>;
}
