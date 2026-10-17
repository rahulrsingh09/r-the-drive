import React from 'react';
import { Link } from '@tanstack/react-router';
import { batterySections } from './navigationModel';

export function RBatteryNavigation({ pathname }: { pathname: string }) {
  if (!batterySections.some(section => section.to === pathname)) return null;
  return <nav className="r-battery-navigation" aria-label="Battery sections">
    {batterySections.map(section => <Link key={section.to} to={section.to} activeOptions={{ exact: true }}>{section.label}</Link>)}
  </nav>;
}
