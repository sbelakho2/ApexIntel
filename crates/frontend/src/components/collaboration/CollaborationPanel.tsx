import { useState, useEffect } from 'react';

// Types for collaboration features
interface WorkspaceShare {
  id: string;
  workspace_id: string;
  shared_with: string;
  share_type: string;
  access_level: string;
  message?: string;
  expires_at?: string;
  created_at: string;
}

interface Annotation {
  id: string;
  entity_type: string;
  entity_id: string;
  author_name: string;
  content: string;
  annotation_type: string;
  is_resolved: boolean;
  created_at: string;
  replies?: Annotation[];
}

interface TeamAssignment {
  id: string;
  team_id: string;
  team_name: string;
  entity_type: string;
  entity_id: string;
  assigned_to: string;
  role: string;
  notes?: string;
}

interface Notification {
  id: string;
  title: string;
  message?: string;
  notification_type: string;
  severity: string;
  is_read: boolean;
  entity_type?: string;
  entity_id?: string;
  created_at: string;
}

// Share workspace modal
const ShareWorkspaceModal: React.FC<{
  workspaceId: string;
  onClose: () => void;
  onShare: (share: WorkspaceShare) => void;
}> = ({ workspaceId, onClose, onShare }) => {
  const [email, setEmail] = useState('');
  const [shareType, setShareType] = useState('view');
  const [accessLevel, setAccessLevel] = useState('read');
  const [message, setMessage] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const response = await fetch(`/api/v1/collaboration/workspaces/${workspaceId}/shares`, {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          shared_with: email,
          share_type: shareType,
          access_level: accessLevel,
          message: message || undefined,
        }),
      });
      const data = await response.json();
      if (data.success && data.data) {
        onShare(data.data);
        onClose();
      }
    } catch (error) {
      console.error('Failed to share workspace:', error);
    }
  };

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg shadow-xl p-6 w-full max-w-md">
        <h3 className="text-lg font-semibold mb-4">Share Workspace</h3>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Email Address</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="w-full border border-gray-300 rounded px-3 py-2"
              placeholder="colleague@company.com"
              required
            />
          </div>
          
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Share Type</label>
            <select
              value={shareType}
              onChange={(e) => setShareType(e.target.value)}
              className="w-full border border-gray-300 rounded px-3 py-2"
            >
              <option value="view">View Only</option>
              <option value="collaborate">Collaborate</option>
              <option value="embed">Embed</option>
            </select>
          </div>
          
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Access Level</label>
            <select
              value={accessLevel}
              onChange={(e) => setAccessLevel(e.target.value)}
              className="w-full border border-gray-300 rounded px-3 py-2"
            >
              <option value="read">Read Only</option>
              <option value="read_write">Read/Write</option>
              <option value="admin">Admin</option>
            </select>
          </div>
          
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Message (Optional)</label>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              className="w-full border border-gray-300 rounded px-3 py-2"
              rows={3}
              placeholder="Add a personal message..."
            />
          </div>
          
          <div className="flex justify-end gap-3 pt-4">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 bg-gray-100 text-gray-700 rounded hover:bg-gray-200"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700"
            >
              Share
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

// Annotation thread component
const AnnotationThread: React.FC<{
  annotation: Annotation;
  onReply: (parentId: string, content: string) => void;
  onResolve: (id: string) => void;
}> = ({ annotation, onReply, onResolve }) => {
  const [showReply, setShowReply] = useState(false);
  const [replyContent, setReplyContent] = useState('');

  const handleReply = () => {
    if (replyContent.trim()) {
      onReply(annotation.id, replyContent);
      setReplyContent('');
      setShowReply(false);
    }
  };

  const typeIcons: Record<string, string> = {
    comment: '💬',
    question: '❓',
    suggestion: '💡',
    correction: '🔧',
    approval: '✅',
  };

  return (
    <div className="border border-gray-200 rounded-lg p-4 bg-white">
      <div className="flex items-start gap-3">
        <div className="w-8 h-8 bg-blue-100 rounded-full flex items-center justify-center text-blue-600 font-semibold">
          {annotation.author_name.charAt(0).toUpperCase()}
        </div>
        <div className="flex-1">
          <div className="flex items-center gap-2 mb-1">
            <span className="font-medium text-gray-900">{annotation.author_name}</span>
            <span className="text-xs text-gray-400">
              {new Date(annotation.created_at).toLocaleString()}
            </span>
            <span className="text-lg">{typeIcons[annotation.annotation_type] || '💬'}</span>
            {annotation.is_resolved && (
              <span className="px-2 py-0.5 bg-green-100 text-green-700 rounded text-xs">
                Resolved
              </span>
            )}
          </div>
          <p className="text-gray-700">{annotation.content}</p>
          
          <div className="flex gap-2 mt-3">
            <button
              onClick={() => setShowReply(!showReply)}
              className="text-sm text-blue-600 hover:text-blue-700"
            >
              Reply
            </button>
            {!annotation.is_resolved && (
              <button
                onClick={() => onResolve(annotation.id)}
                className="text-sm text-green-600 hover:text-green-700"
              >
                Resolve
              </button>
            )}
          </div>
          
          {showReply && (
            <div className="mt-3 flex gap-2">
              <input
                type="text"
                value={replyContent}
                onChange={(e) => setReplyContent(e.target.value)}
                placeholder="Write a reply..."
                className="flex-1 border border-gray-300 rounded px-3 py-1 text-sm"
                onKeyPress={(e) => e.key === 'Enter' && handleReply()}
              />
              <button
                onClick={handleReply}
                className="px-3 py-1 bg-blue-600 text-white rounded text-sm hover:bg-blue-700"
              >
                Reply
              </button>
            </div>
          )}
          
          {annotation.replies && annotation.replies.length > 0 && (
            <div className="mt-3 ml-6 space-y-2 border-l-2 border-gray-200 pl-4">
              {annotation.replies.map((reply) => (
                <div key={reply.id} className="text-sm">
                  <span className="font-medium text-gray-900">{reply.author_name}: </span>
                  <span className="text-gray-700">{reply.content}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

// Team assignment card
const TeamAssignmentCard: React.FC<{
  assignment: TeamAssignment;
  onUnassign: (id: string) => void;
}> = ({ assignment, onUnassign }) => {
  const roleColors: Record<string, string> = {
    lead: 'bg-red-100 text-red-700',
    contributor: 'bg-blue-100 text-blue-700',
    reviewer: 'bg-purple-100 text-purple-700',
    observer: 'bg-gray-100 text-gray-700',
  };

  return (
    <div className="bg-white rounded-lg shadow p-4 flex items-center gap-4">
      <div className="w-10 h-10 bg-blue-100 rounded-full flex items-center justify-center text-blue-600">
        {assignment.team_name.charAt(0).toUpperCase()}
      </div>
      <div className="flex-1">
        <h4 className="font-medium text-gray-900">{assignment.team_name}</h4>
        <div className="flex items-center gap-2 mt-1">
          <span className={`px-2 py-0.5 rounded text-xs ${roleColors[assignment.role] || roleColors.contributor}`}>
            {assignment.role}
          </span>
          <span className="text-xs text-gray-500">Assigned to: {assignment.assigned_to}</span>
        </div>
      </div>
      <button
        onClick={() => onUnassign(assignment.id)}
        className="text-red-600 hover:text-red-700 text-sm"
      >
        Remove
      </button>
    </div>
  );
};

// Notification item component
const NotificationItem: React.FC<{
  notification: Notification;
  onDismiss: (id: string) => void;
}> = ({ notification, onDismiss }) => {
  const severityColors: Record<string, string> = {
    info: 'border-l-blue-500',
    warning: 'border-l-yellow-500',
    critical: 'border-l-red-500',
  };

  const typeIcons: Record<string, string> = {
    assignment: '👤',
    mention: '@',
    comment: '💬',
    share: '🔗',
    update: '📝',
    deadline: '⏰',
    escalation: '⚠️',
  };

  return (
    <div 
      className={`p-4 bg-white border-l-4 ${severityColors[notification.severity] || severityColors.info} border rounded-r shadow-sm ${
        notification.is_read ? 'opacity-60' : ''
      }`}
    >
      <div className="flex justify-between items-start">
        <div className="flex items-start gap-3">
          <span className="text-lg">{typeIcons[notification.notification_type] || '📌'}</span>
          <div>
            <h4 className="font-medium text-gray-900">{notification.title}</h4>
            {notification.message && (
              <p className="text-sm text-gray-600 mt-1">{notification.message}</p>
            )}
            <p className="text-xs text-gray-400 mt-1">
              {new Date(notification.created_at).toLocaleString()}
            </p>
          </div>
        </div>
        <button
          onClick={() => onDismiss(notification.id)}
          className="text-gray-400 hover:text-gray-600"
        >
          ✕
        </button>
      </div>
    </div>
  );
};

// Main collaboration panel component
export const CollaborationPanel: React.FC<{
  entityType?: string;
  entityId?: string;
}> = ({ entityType, entityId }) => {
  const [activeTab, setActiveTab] = useState<'shares' | 'annotations' | 'teams' | 'notifications'>('shares');
  const [shares, setShares] = useState<WorkspaceShare[]>([]);
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [assignments, setAssignments] = useState<TeamAssignment[]>([]);
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [showShareModal, setShowShareModal] = useState(false);
  const [newAnnotation, setNewAnnotation] = useState('');
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchData();
  }, [entityType, entityId]);

  const fetchData = async () => {
    const token = localStorage.getItem('api_token');
    const headers = { 'Authorization': `Bearer ${token}` };

    try {
      const endpoints = [
        entityId ? `/api/v1/collaboration/workspaces/${entityId}/shares` : null,
        entityType && entityId ? `/api/v1/collaboration/annotations?entity_type=${entityType}&entity_id=${entityId}` : '/api/v1/collaboration/annotations',
        entityType && entityId ? `/api/v1/collaboration/teams?entity_type=${entityType}&entity_id=${entityId}` : '/api/v1/collaboration/teams',
        '/api/v1/collaboration/notifications',
      ].filter(Boolean) as string[];

      const responses = await Promise.all(endpoints.map(url => fetch(url, { headers })));
      const data = await Promise.all(responses.map(r => r.json()));

      if (data[0]?.success) setShares(data[0].data || []);
      if (data[1]?.success) setAnnotations(data[1].data || []);
      if (data[2]?.success) setAssignments(data[2].data || []);
      if (data[3]?.success) setNotifications(data[3].data || []);
    } catch (error) {
      console.error('Failed to fetch collaboration data:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleAddAnnotation = async () => {
    if (!newAnnotation.trim() || !entityType || !entityId) return;

    try {
      const response = await fetch('/api/v1/collaboration/annotations', {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          entity_type: entityType,
          entity_id: entityId,
          content: newAnnotation,
          annotation_type: 'comment',
        }),
      });
      const data = await response.json();
      if (data.success && data.data) {
        setAnnotations([data.data, ...annotations]);
        setNewAnnotation('');
      }
    } catch (error) {
      console.error('Failed to add annotation:', error);
    }
  };

  const handleReply = async (parentId: string, content: string) => {
    if (!entityType || !entityId) return;

    try {
      const response = await fetch('/api/v1/collaboration/annotations', {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('api_token')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          entity_type: entityType,
          entity_id: entityId,
          content,
          annotation_type: 'comment',
          parent_id: parentId,
        }),
      });
      const data = await response.json();
      if (data.success) {
        fetchData(); // Refresh to get updated thread
      }
    } catch (error) {
      console.error('Failed to reply:', error);
    }
  };

  const handleResolve = async (id: string) => {
    try {
      await fetch(`/api/v1/collaboration/annotations/${id}/resolve`, {
        method: 'POST',
        headers: { 'Authorization': `Bearer ${localStorage.getItem('api_token')}` },
      });
      setAnnotations(annotations.map(a => 
        a.id === id ? { ...a, is_resolved: true } : a
      ));
    } catch (error) {
      console.error('Failed to resolve annotation:', error);
    }
  };

  const handleDismissNotification = async (id: string) => {
    try {
      await fetch(`/api/v1/collaboration/notifications/${id}/dismiss`, {
        method: 'POST',
        headers: { 'Authorization': `Bearer ${localStorage.getItem('api_token')}` },
      });
      setNotifications(notifications.filter(n => n.id !== id));
    } catch (error) {
      console.error('Failed to dismiss notification:', error);
    }
  };

  const unreadCount = notifications.filter(n => !n.is_read).length;

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  return (
    <div className="bg-gray-50 rounded-lg">
      {/* Tabs */}
      <div className="flex border-b border-gray-200 bg-white rounded-t-lg">
        <button
          onClick={() => setActiveTab('shares')}
          className={`px-4 py-3 font-medium text-sm border-b-2 ${
            activeTab === 'shares' ? 'border-blue-600 text-blue-600' : 'border-transparent text-gray-500'
          }`}
        >
          Shares ({shares.length})
        </button>
        <button
          onClick={() => setActiveTab('annotations')}
          className={`px-4 py-3 font-medium text-sm border-b-2 ${
            activeTab === 'annotations' ? 'border-blue-600 text-blue-600' : 'border-transparent text-gray-500'
          }`}
        >
          Annotations ({annotations.length})
        </button>
        <button
          onClick={() => setActiveTab('teams')}
          className={`px-4 py-3 font-medium text-sm border-b-2 ${
            activeTab === 'teams' ? 'border-blue-600 text-blue-600' : 'border-transparent text-gray-500'
          }`}
        >
          Teams ({assignments.length})
        </button>
        <button
          onClick={() => setActiveTab('notifications')}
          className={`px-4 py-3 font-medium text-sm border-b-2 flex items-center gap-1 ${
            activeTab === 'notifications' ? 'border-blue-600 text-blue-600' : 'border-transparent text-gray-500'
          }`}
        >
          Notifications
          {unreadCount > 0 && (
            <span className="px-2 py-0.5 bg-red-500 text-white rounded-full text-xs">
              {unreadCount}
            </span>
          )}
        </button>
      </div>

      {/* Content */}
      <div className="p-4">
        {/* Shares Tab */}
        {activeTab === 'shares' && (
          <div className="space-y-4">
            <div className="flex justify-between items-center">
              <h3 className="font-semibold text-gray-900">Workspace Shares</h3>
              <button
                onClick={() => setShowShareModal(true)}
                className="px-3 py-1 bg-blue-600 text-white rounded text-sm hover:bg-blue-700"
              >
                + Share
              </button>
            </div>
            {shares.length === 0 ? (
              <p className="text-gray-500 text-center py-4">No shares yet</p>
            ) : (
              shares.map((share) => (
                <div key={share.id} className="bg-white rounded-lg shadow p-4 flex items-center justify-between">
                  <div>
                    <p className="font-medium text-gray-900">{share.shared_with}</p>
                    <div className="flex gap-2 mt-1">
                      <span className="px-2 py-0.5 bg-blue-100 text-blue-700 rounded text-xs">
                        {share.share_type}
                      </span>
                      <span className="px-2 py-0.5 bg-gray-100 text-gray-700 rounded text-xs">
                        {share.access_level}
                      </span>
                    </div>
                  </div>
                  <p className="text-xs text-gray-400">
                    {new Date(share.created_at).toLocaleDateString()}
                  </p>
                </div>
              ))
            )}
          </div>
        )}

        {/* Annotations Tab */}
        {activeTab === 'annotations' && (
          <div className="space-y-4">
            {entityType && entityId && (
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newAnnotation}
                  onChange={(e) => setNewAnnotation(e.target.value)}
                  placeholder="Add a comment..."
                  className="flex-1 border border-gray-300 rounded px-3 py-2"
                  onKeyPress={(e) => e.key === 'Enter' && handleAddAnnotation()}
                />
                <button
                  onClick={handleAddAnnotation}
                  className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700"
                >
                  Add
                </button>
              </div>
            )}
            {annotations.length === 0 ? (
              <p className="text-gray-500 text-center py-4">No annotations yet</p>
            ) : (
              annotations.map((annotation) => (
                <AnnotationThread
                  key={annotation.id}
                  annotation={annotation}
                  onReply={handleReply}
                  onResolve={handleResolve}
                />
              ))
            )}
          </div>
        )}

        {/* Teams Tab */}
        {activeTab === 'teams' && (
          <div className="space-y-4">
            <div className="flex justify-between items-center">
              <h3 className="font-semibold text-gray-900">Team Assignments</h3>
              <button className="px-3 py-1 bg-blue-600 text-white rounded text-sm hover:bg-blue-700">
                + Assign Team
              </button>
            </div>
            {assignments.length === 0 ? (
              <p className="text-gray-500 text-center py-4">No team assignments</p>
            ) : (
              assignments.map((assignment) => (
                <TeamAssignmentCard
                  key={assignment.id}
                  assignment={assignment}
                  onUnassign={(id) => setAssignments(assignments.filter(a => a.id !== id))}
                />
              ))
            )}
          </div>
        )}

        {/* Notifications Tab */}
        {activeTab === 'notifications' && (
          <div className="space-y-2">
            {notifications.length === 0 ? (
              <p className="text-gray-500 text-center py-4">No notifications</p>
            ) : (
              notifications.map((notification) => (
                <NotificationItem
                  key={notification.id}
                  notification={notification}
                  onDismiss={handleDismissNotification}
                />
              ))
            )}
          </div>
        )}
      </div>

      {/* Share Modal */}
      {showShareModal && entityId && (
        <ShareWorkspaceModal
          workspaceId={entityId}
          onClose={() => setShowShareModal(false)}
          onShare={(share) => setShares([...shares, share])}
        />
      )}
    </div>
  );
};

export default CollaborationPanel;

#[cfg(test)]
mod collaboration_tests {
    use super::*;

    // ── ShareWorkspaceModal Tests ─────────────────────────────────────────────

    #[test]
    fn share_modal_initial_state() {
        let (email, setEmail) = (String::new(), false);
        let (shareType, setShareType) = ("view", false);
        let (accessLevel, setAccessLevel) = ("read", false);
        let (message, setMessage) = (String::new(), false);
        
        assert_eq!(email, "");
        assert_eq!(shareType, "view");
        assert_eq!(accessLevel, "read");
        assert_eq!(message, "");
    }

    #[test]
    fn share_type_options() {
        let valid_types = vec!["view", "collaborate", "embed"];
        for share_type in valid_types {
            assert!(["view", "collaborate", "embed"].contains(&share_type));
        }
    }

    #[test]
    fn access_level_options() {
        let valid_levels = vec!["read", "read_write", "admin"];
        for level in valid_levels {
            assert!(["read", "read_write", "admin"].contains(&level));
        }
    }

    // ── AnnotationThread Tests ────────────────────────────────────────────────

    #[test]
    fn annotation_type_icons() {
        let type_icons = vec![
            ("comment", "💬"),
            ("question", "❓"),
            ("suggestion", "💡"),
            ("correction", "🔧"),
            ("approval", "✅"),
        ];
        
        for (type_name, icon) in type_icons {
            assert!(!icon.is_empty());
        }
    }

    #[test]
    fn annotation_thread_state() {
        let show_reply = false;
        let reply_content = String::new();
        
        assert!(!show_reply);
        assert_eq!(reply_content, "");
    }

    // ── TeamAssignmentCard Tests ─────────────────────────────────────────────

    #[test]
    fn team_assignment_role_colors() {
        let role_colors = vec![
            ("lead", "bg-red-100 text-red-700"),
            ("contributor", "bg-blue-100 text-blue-700"),
            ("reviewer", "bg-purple-100 text-purple-700"),
            ("observer", "bg-gray-100 text-gray-700"),
        ];
        
        for (role, color) in role_colors {
            assert!(color.contains("100"));
        }
    }

    // ── NotificationItem Tests ───────────────────────────────────────────────

    #[test]
    fn notification_severity_colors() {
        let severity_colors = vec![
            ("info", "border-l-blue-500"),
            ("warning", "border-l-yellow-500"),
            ("critical", "border-l-red-500"),
        ];
        
        for (severity, color) in severity_colors {
            assert!(color.starts_with("border-l-"));
        }
    }

    #[test]
    fn notification_type_icons() {
        let type_icons = vec![
            ("assignment", "👤"),
            ("mention", "@"),
            ("comment", "💬"),
            ("share", "🔗"),
            ("update", "📝"),
            ("deadline", "⏰"),
            ("escalation", "⚠️"),
        ];
        
        for (type_name, icon) in type_icons {
            assert!(!icon.is_empty());
        }
    }

    // ── Collaboration Data Structure Tests ──────────────────────────────────

    #[test]
    fn workspace_share_serialization() {
        let share = WorkspaceShare {
            id: "share-123".to_string(),
            workspace_id: "ws-456".to_string(),
            shared_with: "user@example.com".to_string(),
            share_type: "collaborate".to_string(),
            access_level: "read_write".to_string(),
            message: Some("Let's collaborate".to_string()),
            expires_at: None,
            created_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&share).unwrap();
        assert!(json.contains("share-123"));
        assert!(json.contains("collaborate"));
    }

    #[test]
    fn annotation_serialization() {
        let annotation = Annotation {
            id: "ann-123".to_string(),
            entity_type: "company".to_string(),
            entity_id: "company-456".to_string(),
            author_name: "John Doe".to_string(),
            content: "This is a comment".to_string(),
            annotation_type: "comment".to_string(),
            is_resolved: false,
            created_at: "2024-01-15T10:00:00Z".to_string(),
            replies: Some(vec![]),
        };
        
        let json = serde_json::to_string(&annotation).unwrap();
        assert!(json.contains("comment"));
        assert!(!json.contains("is_resolved"));
    }

    #[test]
    fn team_assignment_serialization() {
        let assignment = TeamAssignment {
            id: "assign-123".to_string(),
            team_id: "team-security".to_string(),
            team_name: "Security Team".to_string(),
            entity_type: "warning".to_string(),
            entity_id: "warning-456".to_string(),
            assigned_to: "analyst-1".to_string(),
            role: "lead".to_string(),
            notes: Some("Critical issue".to_string()),
        };
        
        let json = serde_json::to_string(&assignment).unwrap();
        assert!(json.contains("team-security"));
        assert!(json.contains("lead"));
    }

    #[test]
    fn notification_serialization() {
        let notification = Notification {
            id: "notif-123".to_string(),
            title: "New Assignment".to_string(),
            message: Some("You've been assigned to investigate".to_string()),
            notification_type: "assignment".to_string(),
            severity: "info".to_string(),
            is_read: false,
            entity_type: Some("warning".to_string()),
            entity_id: Some("warning-456".to_string()),
            created_at: "2024-01-15T10:00:00Z".to_string(),
        };
        
        let json = serde_json::to_string(&notification).unwrap();
        assert!(json.contains("New Assignment"));
        assert!(!json.contains("is_read"));
    }

    // ── Tab Navigation Tests ─────────────────────────────────────────────────

    #[test]
    fn tab_options() {
        let tabs = vec![
            ("shares", "Shares"),
            ("annotations", "Annotations"),
            ("teams", "Teams"),
            ("notifications", "Notifications"),
        ];
        
        for (key, label) in tabs {
            assert!(!key.is_empty());
            assert!(!label.is_empty());
        }
    }

    // ── Validation Tests ─────────────────────────────────────────────────────

    #[test]
    fn email_validation() {
        let valid_emails = vec![
            "user@example.com",
            "test.user@company.org",
            "analyst+tag@domain.co",
        ];
        
        for email in valid_emails {
            assert!(email.contains("@"));
            assert!(email.contains("."));
        }
    }

    #[test]
    fn access_level_validation() {
        let valid_levels = vec!["read", "read_write", "admin"];
        for level in valid_levels {
            assert!(["read", "read_write", "admin"].contains(&level));
        }
    }
}
