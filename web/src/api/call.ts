import { ApiError } from "./errors";

type Answer<T> = { data?: T; error?: unknown; response: Response };

/**
 * Awaits an openapi-fetch request and returns what it answered, or throws an
 * ApiError for any status outside 2xx. A network failure rejects as fetch
 * does, with a TypeError.
 */
export async function call<T>(request: Promise<Answer<T>>): Promise<T> {
  const { data, error, response } = await request;
  if (!response.ok) {
    throw ApiError.from(response, error);
  }
  return data as T;
}

/** Like call, for routes that answer 204 with no body. */
export async function send(request: Promise<Answer<unknown>>): Promise<void> {
  const { error, response } = await request;
  if (!response.ok) {
    throw ApiError.from(response, error);
  }
}
