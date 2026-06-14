// Analyst View Route
// Phase 4.3: User Experience Enhancement

import AnalystView from '../components/analyst/AnalystView';

export const analystRoute = {
  path: '/analyst',
  component: AnalystView,
  meta: {
    title: 'Analyst View',
    description: 'Investigation workspace, entity relationship explorer, and source evidence viewer',
    roles: ['analyst', 'admin'],
  },
};

export const analystRoutes = [
  {
    path: '/analyst',
    name: 'Investigation Workspace',
    component: AnalystView,
    exact: true,
  },
  {
    path: '/analyst/workspaces',
    name: 'Workspaces',
    component: AnalystView,
  },
  {
    path: '/analyst/entities',
    name: 'Entity Explorer',
    component: AnalystView,
  },
  {
    path: '/analyst/evidence',
    name: 'Source Evidence',
    component: AnalystView,
  },
];

export default AnalystView;
