import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useMe } from '@panda/hooks';
import { RAppLayout } from './RAppLayout';
import { canManagePeople } from './navigationModel';

export function RExplore() {
  const { data: me } = useMe();
  return (
    <RAppLayout activeKey="explore">
      <div className="r-page-heading"><h1>Explore.</h1><p>The rest of your Rivian, in one place.</p></div>
      <div className="r-directory">
        <Link to="/efficiency"><span><strong>Efficiency</strong><small>Trends, temperature and trip comparisons</small></span><ArrowRight /></Link>
        <Link to="/vehicle-health"><span><strong>Vehicle health</strong><small>Readings, tires, software and connection</small></span><ArrowRight /></Link>
        <Link to="/battery"><span><strong>Battery</strong><small>State of charge and parked energy</small></span><ArrowRight /></Link>
        <Link to="/battery/phantom-drain"><span><strong>Phantom drain</strong><small>Parked energy, sleep and battery loss</small></span><ArrowRight /></Link>
        <Link to="/settings" search={{ section: 'dashboards' }}><span><strong>Dashboard library</strong><small>Your views and saved layouts</small></span><ArrowRight /></Link>
        <Link to="/settings"><span><strong>All settings</strong><small>Preferences, connections and administration</small></span><ArrowRight /></Link>
        {canManagePeople(me?.role) && (
          <Link to="/users"><span><strong>People & access</strong><small>Accounts, invitations and permissions</small></span><ArrowRight /></Link>
        )}
      </div>
    </RAppLayout>
  );
}
