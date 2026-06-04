"""GraphQL query and mutation text."""
from __future__ import annotations


def review_threads_query() -> str:
    return """
    query ReviewThreads($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $repo) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor) {
            pageInfo { hasNextPage endCursor }
            nodes {
              id isResolved path line startLine originalLine
              comments(first: 100) { nodes { id body author { login } url path line commit { oid } originalCommit { oid } createdAt updatedAt } }
            }
          }
        }
      }
    }
    """


def reply_to_thread_mutation() -> str:
    return """
    mutation ReplyToThread($threadId: ID!, $body: String!) {
      addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $threadId, body: $body}) {
        comment { id url }
      }
    }
    """


def resolve_thread_mutation() -> str:
    return """
    mutation ResolveThread($threadId: ID!) {
      resolveReviewThread(input: {threadId: $threadId}) { thread { id isResolved } }
    }
    """
