import React from 'react';
import { Link } from '@tanstack/react-router';

export function RBatteryNavigation({ pathname }: { pathname: string }) {
  if (pathname !== '/battery' && pathname !== '/battery/phantom-drain') return null;
  return <nav className="r-battery-navigation" aria-label="Battery sections">
    <Link to="/battery" activeOptions={{ exact: true }}>Battery overview</Link>
    <Link to="/battery/phantom-drain" activeOptions={{ exact: true }}>Phantom drain</Link>
  </nav>;
}
