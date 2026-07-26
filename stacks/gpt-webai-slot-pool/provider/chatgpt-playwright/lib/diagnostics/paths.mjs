
import path from 'node:path';
import process from 'node:process';

export function hostPathFor(containerPath) {
  const containerRoot = process.env.GPT_WEBAI_ARTIFACTS_DIR || '';
  const hostRoot = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR || containerRoot;
  if (!containerRoot || !hostRoot || !containerPath.startsWith(containerRoot)) return containerPath;
  return path.join(hostRoot, path.relative(containerRoot, containerPath));
}
