type AssetsBinding = {
  fetch(request: Request): Promise<Response>;
};

type WorkerEnvironment = {
  ASSETS: AssetsBinding;
};

export default {
  async fetch(request: Request, environment: WorkerEnvironment): Promise<Response> {
    const response = await environment.ASSETS.fetch(request);
    if (response.status !== 404 || request.method !== "GET") return response;

    const fallbackUrl = new URL(request.url);
    fallbackUrl.pathname = "/index.html";
    return environment.ASSETS.fetch(new Request(fallbackUrl, request));
  },
};
