// ApexIntel Frontend Routes - Phase 4.3 User Experience Enhancement
// Route definitions for Executive, Analyst, and Operational views

import { lazy } from 'react';

// Lazy load components for code splitting
const ExecutiveView = lazy(() => import('./components/executive/ExecutiveView'));
const AnalystView = lazy(() => import('./components/analyst/AnalystView'));
const OperationalView = lazy(() => import('./components/operational/OperationalView'));
const CollaborationPanel = lazy(() => import('./components/collaboration/CollaborationPanel'));

export interface RouteDefinition {
  path: string;
  name: string;
  component: React.LazyExoticComponent<React.ComponentType<any>>;
  exact?: boolean;
  meta?: {
    title: string;
    description: string;
    roles?: string[];
    requiresAuth?: boolean;
  };
}

// Main application routes
export const appRoutes: RouteDefinition[] = [
  // Executive Dashboard Routes
  {
    path: '/executive',
    name: 'Executive Dashboard',
    component: ExecutiveView,
    exact: true,
    meta: {
      title: 'Executive Dashboard',
      description: 'Strategic overview with top opportunities, critical threats, and recommended actions',
      roles: ['executive', 'admin', 'analyst'],
      requiresAuth: true,
    },
  },
  {
    path: '/executive/opportunities',
    name: 'Strategic Opportunities',
    component: ExecutiveView,
    meta: {
      title: 'Strategic Opportunities',
      description: 'View and manage top strategic opportunities',
      roles: ['executive', 'admin', 'analyst'],
      requiresAuth: true,
    },
  },
  {
    path: '/executive/threats',
    name: 'Critical Threats',
    component: ExecutiveView,
    meta: {
      title: 'Critical Threats',
      description: 'Monitor and address critical threats requiring attention',
      roles: ['executive', 'admin', 'analyst'],
      requiresAuth: true,
    },
  },
  {
    path: '/executive/actions',
    name: 'Recommended Actions',
    component: ExecutiveView,
    meta: {
      title: 'Recommended Actions',
      description: 'Prioritized recommended actions based on analysis',
      roles: ['executive', 'admin', 'analyst'],
      requiresAuth: true,
    },
  },

  // Analyst View Routes
  {
    path: '/analyst',
    name: 'Analyst View',
    component: AnalystView,
    exact: true,
    meta: {
      title: 'Analyst View',
      description: 'Investigation workspace, entity relationship explorer, and source evidence viewer',
      roles: ['analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/analyst/workspaces',
    name: 'Investigation Workspaces',
    component: AnalystView,
    meta: {
      title: 'Investigation Workspaces',
      description: 'Manage investigation workspaces',
      roles: ['analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/analyst/entities',
    name: 'Entity Explorer',
    component: AnalystView,
    meta: {
      title: 'Entity Explorer',
      description: 'Explore entity relationships and connections',
      roles: ['analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/analyst/evidence',
    name: 'Source Evidence',
    component: AnalystView,
    meta: {
      title: 'Source Evidence',
      description: 'View and manage source evidence for investigations',
      roles: ['analyst', 'admin'],
      requiresAuth: true,
    },
  },

  // Operational View Routes
  {
    path: '/operational',
    name: 'Operational View',
    component: OperationalView,
    exact: true,
    meta: {
      title: 'Operational View',
      description: 'Daily operations including priority queue, supplier risks, and alerts',
      roles: ['operator', 'analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/operational/queue',
    name: 'Priority Queue',
    component: OperationalView,
    meta: {
      title: 'Priority Queue',
      description: 'Manage daily priority queue items',
      roles: ['operator', 'analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/operational/suppliers',
    name: 'Supplier Risks',
    component: OperationalView,
    meta: {
      title: 'Supplier Risks',
      description: 'Monitor supplier risk levels and mitigations',
      roles: ['operator', 'analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/operational/pipeline',
    name: 'Pipeline Tracker',
    component: OperationalView,
    meta: {
      title: 'Pipeline Tracker',
      description: 'Track pipeline opportunities through stages',
      roles: ['operator', 'analyst', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/operational/alerts',
    name: 'Alert Management',
    component: OperationalView,
    meta: {
      title: 'Alert Management',
      description: 'Manage and acknowledge system alerts',
      roles: ['operator', 'analyst', 'admin'],
      requiresAuth: true,
    },
  },

  // Collaboration Routes
  {
    path: '/collaboration',
    name: 'Collaboration',
    component: CollaborationPanel,
    exact: true,
    meta: {
      title: 'Collaboration',
      description: 'Investigation sharing, annotations, team assignments, and activity feed',
      roles: ['analyst', 'operator', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/collaboration/shares',
    name: 'Shared Workspaces',
    component: CollaborationPanel,
    meta: {
      title: 'Shared Workspaces',
      description: 'View and manage shared investigation workspaces',
      roles: ['analyst', 'operator', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/collaboration/annotations',
    name: 'Annotations',
    component: CollaborationPanel,
    meta: {
      title: 'Annotations',
      description: 'View and add annotations to investigations',
      roles: ['analyst', 'operator', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/collaboration/teams',
    name: 'Team Assignments',
    component: CollaborationPanel,
    meta: {
      title: 'Team Assignments',
      description: 'Manage team assignments for investigations',
      roles: ['analyst', 'operator', 'admin'],
      requiresAuth: true,
    },
  },
  {
    path: '/collaboration/activity',
    name: 'Activity Feed',
    component: CollaborationPanel,
    meta: {
      title: 'Activity Feed',
      description: 'View recent activity across investigations',
      roles: ['analyst', 'operator', 'admin'],
      requiresAuth: true,
    },
  },
];

// Get route by path
export function getRouteByPath(path: string): RouteDefinition | undefined {
  return appRoutes.find(route => 
    route.exact ? route.path === path : path.startsWith(route.path)
  );
}

// Get routes by role
export function getRoutesByRole(role: string): RouteDefinition[] {
  return appRoutes.filter(route => 
    !route.meta?.roles || route.meta.roles.includes(role)
  );
}

// Navigation items for menu
export const navigationItems = [
  {
    section: 'Executive',
    routes: appRoutes.filter(r => r.path.startsWith('/executive')),
  },
  {
    section: 'Analyst',
    routes: appRoutes.filter(r => r.path.startsWith('/analyst')),
  },
  {
    section: 'Operational',
    routes: appRoutes.filter(r => r.path.startsWith('/operational')),
  },
  {
    section: 'Collaboration',
    routes: appRoutes.filter(r => r.path.startsWith('/collaboration')),
  },
];

export default appRoutes;
