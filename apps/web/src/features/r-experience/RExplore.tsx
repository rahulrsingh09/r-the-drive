import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useMe } from '@panda/hooks';
import { RAppLayout } from './RAppLayout';
import { exploreDirectory } from './navigationModel';

export function RExplore() {
  const { data: me } = useMe();
  const sections = exploreDirectory(me?.role);
  return (
    <RAppLayout activeKey="explore">
      <div className="r-page-heading"><h1>Explore.</h1><p>The rest of your Rivian, in one place.</p></div>
      <div className="r-directory">
        {sections.map(section => <Link key={section.title} to={section.to} {...('search' in section ? { search: section.search } : {})}>
          <span><strong>{section.title}</strong><small>{section.description}</small></span><ArrowRight />
        </Link>)}
      </div>
    </RAppLayout>
  );
}
