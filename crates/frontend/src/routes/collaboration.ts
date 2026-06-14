// Collaboration Routes
// Phase 4.3: User Experience Enhancement

import CollaborationPanel from '../components/collaboration/CollaborationPanel';

export const collaborationRoute = {
  path: '/collaboration',
  component: CollaborationPanel,
  meta: {
    title: 'Collaboration',
    description: 'Investigation sharing, annotations, team assignments, and activity feed',
    roles: ['analyst', 'operator', 'admin'],
  },
};

export const collaborationRoutes = [
  {
    path: '/collaboration',
    name: 'Collaboration',
    component: CollaborationPanel,
    exact: true,
  },
  {
    path: '/collaboration/shares',
    name: 'Shared Workspaces',
    component: CollaborationPanel,
  },
  {
    path: '/collaboration/annotations',
    name: 'Annotations',
    component: CollaborationPanel,
  },
  {
    path: '/collaboration/teams',
    name: 'Team Assignments',
    component: CollaborationPanel,
  },
  {
    path: '/collaboration/activity',
    name: 'Activity Feed',
    component: CollaborationPanel,
  },
];

export default CollaborationPanel;
