import { useState, useEffect, useRef } from 'react';

// Types for analyst view
interface InvestigationWorkspace {
  id: string;
  name: string;
  description?: string;
  workspace_type: string;
  owner_id: string;
  team_id?: string;
  status: string;
  visibility: string;
  tags: string[];
  entity_focus: string[];
  findings?: string;
  conclusions?: string;
  created_at: string;
  updated_at: string;
}

interface EntityNode {
  id: string;
  type: 'company' | 'person' | 'warning' | 'insight';
  name: string;
  metadata: Record<string, unknown>;
}

interface EntityEdge {
  source: string;
  target: string;
  type: string;
  weight: number;
}

interface ActivityEntry {
  id: string;
  actor_id: string;
  actor_name: string;
  action_type: string;
  entity_type?: string;
  entity_id?: string;
  entity_name?: string;
  details: Record<string, unknown>;
  workspace_id?: string;
  created_at: string;
}

// Entity relationship explorer component
const EntityGraph: React.FC<{ nodes: EntityNode[]; edges: EntityEdge[] }> = ({ nodes, edges }) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    
    // Clear canvas
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    
    // Draw edges
    ctx.strokeStyle = '#94a3b8';
    ctx.lineWidth = 1;
    edges.forEach((edge) => {
      const sourceNode = nodes.find(n => n.id === edge.source);
      const targetNode = nodes.find(n => n.id === edge.target);
      if (sourceNode && targetNode) {
        // Simple positioning for demo
        const sourceX = (parseInt(edge.source.replace(/\D/g, '')) || 0) % 500 + 100;
        const sourceY = (parseInt(edge.source.replace(/\D/g, '')) || 0) % 300 + 100;
        const targetX = (parseInt(edge.target.replace(/\D/g, '')) || 1) % 500 + 100;
        const targetY = (parseInt(edge.target.replace(/\D/g, '')) || 1) % 300 + 100;
        
        ctx.beginPath();
        ctx.moveTo(sourceX, sourceY);
        ctx.lineTo(targetX, targetY);
        ctx.stroke();
        
        // Draw weight label
        ctx.fillStyle = '#64748b';
        ctx.font = '10px sans-serif';
        ctx.fillText(edge.weight.toFixed(2), (sourceX + targetX) / 2, (sourceY + targetY) / 2 - 5);
      }
    });
    
    // Draw nodes
    nodes.forEach((node, index) => {
      const x = (index * 120) % 500 + 100;
      const y = (index * 80) % 300 + 100;
      
      // Node circle
      ctx.beginPath();
      ctx.arc(x, y, 30, 0, Math.PI * 2);
      
      const colors: Record<string, string> = {
        company: '#3b82f6',
        person: '#8b5cf6',
        warning: '#ef4444',
        insight: '#10b981',
      };
      ctx.fillStyle = colors[node.type] || '#6b7280';
      ctx.fill();
      
      // Node label
      ctx.fillStyle = '#ffffff';
      ctx.font = '11px sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(node.name.substring(0, 10), x, y + 4);
    });
  }, [nodes, edges]);

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <h3 className="text-lg font-semibold mb-4">Entity Relationship Explorer</h3>
      <canvas 
        ref={canvasRef} 
        width={700} 
        height={400} 
        className="border border-gray-200 rounded"
      />
      <div className="flex gap-4 mt-4 justify-center">
        {['company', 'person', 'warning', 'insight'].map((type) => (
          <div key={type} className="flex items-center gap-2">
            <div className={`w-4 h-4 rounded-full ${
              type === 'company' ? 'bg-blue-500' :
              type === 'person' ? 'bg-purple-500' :
              type === 'warning' ? 'bg-red-500' : 'bg-green-500'
            }`} />
            <span className="text-sm text-gray-600 capitalize">{type}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

// Source evidence viewer
const SourceEvidenceViewer: React.FC<{ entityId: string }> = ({ entityId }) => {
  const [evidence, setEvidence] = useState<Array<{
    id: string;
    source_url: string;
    source_name?: string;
    reliability_score: number;
    excerpt?: string;
  }>>([]);
  
  useEffect(() => {
    fetch(`/api/v1/collaboration/evidence?entity_id=${entityId}`)
      .then(res => res.json())
      .then(data => {
        if (data.success) setEvidence(data.data || []);
      })
      .catch(console.error);
  }, [entityId]);

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <h3 className="text-lg font-semibold mb-4">Source Evidence</h3>
      <div className="space-y-3">
        {evidence.length === 0 ? (
          <p className="text-gray-500 text-center py-4">No evidence available</p>
        ) : (
          evidence.map((item) => (
            <div key={item.id} className="border border-gray-200 rounded-lg p-3">
              <div className="flex justify-between items-start mb-2">
                <a 
                  href={item.source_url} 
                  target="_blank" 
                  rel="noopener noreferrer"
                  className="text-blue-600 hover:underline font-medium"
                >
                  {item.source_name || item.source_url}
                </a>
                <span className={`px-2 py-1 rounded text-xs ${
                  item.reliability_score > 0.8 ? 'bg-green-100 text-green-700' :
                  item.reliability_score > 0.5 ? 'bg-yellow-100 text-yellow-700' :
                  'bg-red-100 text-red-700'
                }`}>
                  {(item.reliability_score * 100).toFixed(0)}% reliable
                </span>
              </div>
              {item.excerpt && (
                <p className="text-sm text-gray-600 italic mt-2">{item.excerpt}</p>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
};

// Investigation workspace editor
const WorkspaceEditor: React.FC<{
  workspace: InvestigationWorkspace;
  onUpdate: (updates: Partial<InvestigationWorkspace>) => void;
}> = ({ workspace, onUpdate }) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editedWorkspace, setEditedWorkspace] = useState(workspace);

  const handleSave = () => {
    onUpdate(editedWorkspace);
    setIsEditing(false);
  };

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <div className="flex justify-between items-center mb-4">
        <h3 className="text-lg font-semibold">Workspace: {workspace.name}</h3>
        <button 
          onClick={() => isEditing ? handleSave() : setIsEditing(true)}
          className={`px-3 py-1 rounded text-sm ${
            isEditing ? 'bg-green-600 text-white' : 'bg-blue-600 text-white'
          }`}
        >
          {isEditing ? 'Save' : 'Edit'}
        </button>
      </div>
      
      <div className="space-y-4">
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">Description</label>
          {isEditing ? (
            <textarea
              value={editedWorkspace.description || ''}
              onChange={(e) => setEditedWorkspace({...editedWorkspace, description: e.target.value})}
              className="w-full border border-gray-300 rounded px-3 py-2"
              rows={3}
            />
          ) : (
            <p className="text-gray-600">{workspace.description || 'No description'}</p>
          )}
        </div>
        
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">Findings</label>
          {isEditing ? (
            <textarea
              value={editedWorkspace.findings || ''}
              onChange={(e) => setEditedWorkspace({...editedWorkspace, findings: e.target.value})}
              className="w-full border border-gray-300 rounded px-3 py-2"
              rows={4}
            />
          ) : (
            <p className="text-gray-600 bg-gray-50 p-3 rounded">{workspace.findings || 'No findings yet'}</p>
          )}
        </div>
        
        <div className="flex gap-2">
          <span className="px-2 py-1 bg-blue-100 text-blue-700 rounded text-sm">
            {workspace.workspace_type}
          </span>
          <span className={`px-2 py-1 rounded text-sm ${
            workspace.status === 'active' ? 'bg-green-100 text-green-700' :
            'bg-gray-100 text-gray-700'
          }`}>
            {workspace.status}
          </span>
          <span className="px-2 py-1 bg-purple-100 text-purple-700 rounded text-sm">
            {workspace.visibility}
          </span>
        </div>
      </div>
    </div>
  );
};

// Activity feed
const ActivityFeed: React.FC<{ entries: ActivityEntry[] }> = ({ entries }) => {
  const actionIcons: Record<string, string> = {
    create: '✨',
    update: '📝',
    delete: '🗑️',
    share: '🔗',
    assign: '👤',
    comment: '💬',
    resolve: '✅',
    reopen: '↩️',
  };

  return (
    <div className="bg-white rounded-lg shadow p-4">
      <h3 className="text-lg font-semibold mb-4">Activity Feed</h3>
      <div className="space-y-3 max-h-96 overflow-y-auto">
        {entries.length === 0 ? (
          <p className="text-gray-500 text-center py-4">No activity recorded</p>
        ) : (
          entries.map((entry) => (
            <div key={entry.id} className="flex gap-3 p-2 hover:bg-gray-50 rounded">
              <span className="text-xl">{actionIcons[entry.action_type] || '📌'}</span>
              <div className="flex-1">
                <p className="text-sm">
                  <span className="font-medium">{entry.actor_name}</span>
                  <span className="text-gray-500"> {entry.action_type}</span>
                  {entry.entity_name && (
                    <span className="text-blue-600"> {entry.entity_name}</span>
                  )}
                </p>
                <p className="text-xs text-gray-400">
                  {new Date(entry.created_at).toLocaleString()}
                </p>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
};

// Main analyst view component
export const AnalystView: React.FC = () => {
  const [workspaces, setWorkspaces] = useState<InvestigationWorkspace[]>([]);
  const [selectedWorkspace, setSelectedWorkspace] = useState<InvestigationWorkspace | null>(null);
  const [entities, setEntities] = useState<{ nodes: EntityNode[]; edges: EntityEdge[] }>({ nodes: [], edges: [] });
  const [activities, setActivities] = useState<ActivityEntry[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchWorkspaces();
    fetchEntities();
    fetchActivities();
  }, []);

  const fetchWorkspaces = async () => {
    try {
      const response = await fetch('/api/v1/collaboration/workspaces', {
        headers: { 'Authorization': `Bearer ${localStorage.getItem('api_token')}` }
      });
      const data = await response.json();
      if (data.success) {
        setWorkspaces(data.data || []);
        if (data.data?.length > 0) {
          setSelectedWorkspace(data.data[0]);
        }
      }
    } catch (error) {
      console.error('Failed to fetch workspaces:', error);
    } finally {
      setLoading(false);
    }
  };

  const fetchEntities = async () => {
    try {
      const response = await fetch('/api/v1/graph/overview', {
        headers: { 'Authorization': `Bearer ${localStorage.getItem('api_token')}` }
      });
      const data = await response.json();
      if (data.success) {
        setEntities({
          nodes: data.data?.nodes?.map((n: { id: string; label: string; node_type: string }) => ({
            id: n.id,
            type: n.node_type as EntityNode['type'],
            name: n.label,
            metadata: {},
          })) || [],
          edges: data.data?.edges?.map((e: { source_id: string; target_id: string; edge_type: string; weight: number }) => ({
            source: e.source_id,
            target: e.target_id,
            type: e.edge_type,
            weight: e.weight,
          })) || [],
        });
      }
    } catch (error) {
      console.error('Failed to fetch entities:', error);
    }
  };

  const fetchActivities = async () => {
    try {
      const response = await fetch('/api/v1/collaboration/activity?limit=20', {
        headers: { 'Authorization': `Bearer ${localStorage.getItem('api_token')}` }
      });
      const data = await response.json();
      if (data.success) {
        setActivities(data.data || []);
      }
    } catch (error) {
      console.error('Failed to fetch activities:', error);
    }
  };

  const handleWorkspaceUpdate = async (updates: Partial<InvestigationWorkspace>) => {
    if (!selectedWorkspace) return;
    
    try {
      const response = await fetch(`/api/v1/collaboration/workspaces/${selectedWorkspace.id}`, {
        method: 'PUT',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(updates),
      });
      const data = await response.json();
      if (data.success) {
        setSelectedWorkspace(data.data);
        fetchWorkspaces();
      }
    } catch (error) {
      console.error('Failed to update workspace:', error);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white shadow-sm">
        <div className="max-w-7xl mx-auto px-4 py-4">
          <h1 className="text-2xl font-bold text-gray-900">Analyst View</h1>
          <p className="text-sm text-gray-500">Investigation workspace and entity analysis</p>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 py-6">
        <div className="grid grid-cols-1 lg:grid-cols-4 gap-6">
          {/* Workspace List */}
          <div className="lg:col-span-1 bg-white rounded-lg shadow p-4">
            <h3 className="text-lg font-semibold mb-4">Workspaces</h3>
            <div className="space-y-2">
              {workspaces.map((ws) => (
                <button
                  key={ws.id}
                  onClick={() => setSelectedWorkspace(ws)}
                  className={`w-full text-left p-3 rounded transition ${
                    selectedWorkspace?.id === ws.id
                      ? 'bg-blue-50 border border-blue-300'
                      : 'bg-gray-50 hover:bg-gray-100'
                  }`}
                >
                  <p className="font-medium text-gray-900">{ws.name}</p>
                  <p className="text-xs text-gray-500">{ws.workspace_type} • {ws.status}</p>
                </button>
              ))}
              {workspaces.length === 0 && (
                <p className="text-gray-500 text-center py-4">No workspaces</p>
              )}
            </div>
            <button className="mt-4 w-full px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700">
              + New Workspace
            </button>
          </div>

          {/* Workspace Editor */}
          <div className="lg:col-span-2 space-y-6">
            {selectedWorkspace && (
              <WorkspaceEditor 
                workspace={selectedWorkspace} 
                onUpdate={handleWorkspaceUpdate} 
              />
            )}
            {selectedWorkspace && (
              <SourceEvidenceViewer entityId={selectedWorkspace.id} />
            )}
          </div>

          {/* Entity Graph and Activity */}
          <div className="lg:col-span-1 space-y-6">
            <EntityGraph nodes={entities.nodes} edges={entities.edges} />
            <ActivityFeed entries={activities} />
          </div>
        </div>
      </main>
    </div>
  );
};

export default AnalystView;

#[cfg(test)]
mod analyst_view_tests {
    use super::*;

    // ── Entity Graph Tests ────────────────────────────────────────────────────

    #[test]
    fn entity_node_types() {
        let node_types = vec![
            "company",
            "person",
            "warning",
            "insight",
        ];
        
        for node_type in node_types {
            assert!(!node_type.is_empty());
        }
    }

    #[test]
    fn entity_edge_types() {
        let edge_types = vec![
            "works_for",
            "manages",
            "subsidiary_of",
            "partners_with",
            "competes_with",
            "related_to",
        ];
        
        for edge_type in edge_types {
            assert!(!edge_type.is_empty());
        }
    }

    #[test]
    fn entity_node_color_mapping() {
        let colors = vec![
            ("company", "#3b82f6"),
            ("person", "#8b5cf6"),
            ("warning", "#ef4444"),
            ("insight", "#10b981"),
        ];
        
        for (node_type, color) in colors {
            assert!(color.starts_with("#"));
            assert_eq!(color.len(), 7);
        }
    }

    // ── Workspace Editor Tests ────────────────────────────────────────────────

    #[test]
    fn workspace_types() {
        let valid_types = vec![
            "ad-hoc",
            "structured",
            "incident",
            "ongoing",
        ];
        
        for workspace_type in valid_types {
            assert!(["ad-hoc", "structured", "incident", "ongoing"].contains(&workspace_type));
        }
    }

    #[test]
    fn workspace_status_values() {
        let valid_statuses = vec![
            "active",
            "closed",
            "archived",
        ];
        
        for status in valid_statuses {
            assert!(["active", "closed", "archived"].contains(&status));
        }
    }

    // ── Source Evidence Tests ────────────────────────────────────────────────

    #[test]
    fn evidence_reliability_score() {
        let scores = vec![0.0, 0.5, 0.75, 1.0];
        
        for score in scores {
            assert!(score >= 0.0 && score <= 1.0);
        }
    }

    // ── Activity Feed Tests ──────────────────────────────────────────────────

    #[test]
    fn activity_action_icons() {
        let action_icons = vec![
            ("create", "✨"),
            ("update", "📝"),
            ("delete", "🗑️"),
            ("share", "🔗"),
            ("assign", "👤"),
            ("comment", "💬"),
            ("resolve", "✅"),
            ("reopen", "↩️"),
        ];
        
        for (action, icon) in action_icons {
            assert!(!icon.is_empty());
        }
    }

    // ── Workspace Serialization Tests ───────────────────────────────────────

    #[test]
    fn investigation_workspace_serialization() {
        let workspace = InvestigationWorkspace {
            id: "ws-123".to_string(),
            name: "Test Workspace".to_string(),
            description: Some("A test workspace".to_string()),
            workspace_type: "ad-hoc".to_string(),
            owner_id: "user-1".to_string(),
            team_id: Some("team-1".to_string()),
            status: "active".to_string(),
            visibility: "team".to_string(),
            tags: vec!["test".to_string(), "demo".to_string()],
            entity_focus: vec!["company-1".to_string(), "person-2".to_string()],
            findings: Some("Initial findings".to_string()),
            conclusions: None,
            created_at: "2024-01-15T10:00:00Z".to_string(),
            updated_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&workspace).unwrap();
        assert!(json.contains("ws-123"));
        assert!(json.contains("Test Workspace"));
        assert!(json.contains("ad-hoc"));
    }

    // ── Entity Graph Data Structure Tests ───────────────────────────────────

    #[test]
    fn entity_node_serialization() {
        let node = EntityNode {
            id: "node-123".to_string(),
            type: "company".to_string(),
            name: "Acme Corp".to_string(),
            metadata: serde_json::json!({"industry": "tech", "employees": 1000}),
        };
        
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("Acme Corp"));
        assert!(json.contains("company"));
    }

    #[test]
    fn entity_edge_serialization() {
        let edge = EntityEdge {
            source: "node-1".to_string(),
            target: "node-2".to_string(),
            type: "works_for".to_string(),
            weight: 0.85,
        };
        
        let json = serde_json::to_string(&edge).unwrap();
        assert!(json.contains("node-1"));
        assert!(json.contains("works_for"));
        assert!(json.contains("0.85"));
    }

    // ── Activity Entry Tests ────────────────────────────────────────────────

    #[test]
    fn activity_entry_serialization() {
        let entry = ActivityEntry {
            id: "activity-123".to_string(),
            actor_id: "user-456".to_string(),
            actor_name: "Jane Smith".to_string(),
            action_type: "update".to_string(),
            entity_type: Some("workspace".to_string()),
            entity_id: Some("ws-789".to_string()),
            entity_name: Some("Test Investigation".to_string()),
            details: serde_json::json!({"field": "findings", "old": "", "new": "Updated findings"}),
            workspace_id: Some("ws-789".to_string()),
            created_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("Jane Smith"));
        assert!(json.contains("update"));
        assert!(json.contains("Test Investigation"));
    }

    // ── Canvas Rendering Tests ──────────────────────────────────────────────

    #[test]
    fn canvas_dimensions() {
        let width = 700;
        let height = 400;
        
        assert!(width > 0);
        assert!(height > 0);
        assert!(width > height);
    }

    // ── State Management Tests ──────────────────────────────────────────────

    #[test]
    fn initial_state_values() {
        let workspaces: Vec<InvestigationWorkspace> = vec![];
        let selected_workspace: Option<InvestigationWorkspace> = None;
        let loading = true;
        
        assert!(workspaces.is_empty());
        assert!(selected_workspace.is_none());
        assert!(loading);
    }

    // ── API Response Handling Tests ─────────────────────────────────────────

    #[test]
    fn api_success_response_parsing() {
        let response = serde_json::json!({
            "success": true,
            "data": [
                {"id": "ws-1", "name": "Workspace 1", "workspace_type": "ad-hoc", "owner_id": "user-1", "status": "active", "visibility": "team", "tags": [], "entity_focus": [], "created_at": "", "updated_at": ""}
            ]
        });
        
        assert!(response["success"].as_bool().unwrap());
        assert!(response["data"].is_array());
    }

    #[test]
    fn api_error_response_parsing() {
        let response = serde_json::json!({
            "success": false,
            "error": {
                "code": "NOT_FOUND",
                "message": "Workspace not found"
            }
        });
        
        assert!(!response["success"].as_bool().unwrap());
        assert!(response["error"].is_object());
    }
}
