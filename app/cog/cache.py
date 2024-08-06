import asyncio
import urllib
from typing import Any, Dict

import aiocache
from starlette.concurrency import run_in_threadpool
from starlette.responses import Response
import redis
import pickle

from fastapi.dependencies.utils import is_coroutine_callable

from app.config import config as appconfig


class cached(aiocache.cached):
    """Custom Cached Decorator."""

    async def get_from_cache(self, key):
        try:
            value = await self.cache.get(key)
            if value:
                print(f"Cache HIT: {key}")
                # Deserialize if the value is stored as bytes
                if isinstance(value, bytes):
                    value = pickle.loads(value)
            else:
                print(f"Cache MISS: {key}")
            if isinstance(value, Response):
                value.headers["X-Cache"] = "HIT"
            return value
        except Exception as e:
            aiocache.logger.exception(
                "Couldn't retrieve %s, unexpected error: %s", key, e
            )
            print(f"Error retrieving from cache: {e}")

    async def set_in_cache(self, key, value):
        try:
            print(f"Setting cache for key: {key} with value: {value}")
            # Serialize the value before storing
            await self.cache.set(key, pickle.dumps(value))
            print(f"Cache SET: {key}")
        except Exception as e:
            aiocache.logger.exception(
                "Couldn't store %s, unexpected error: %s", key, e
            )
            print(f"Error setting cache: {e}")

    async def decorator(
        self,
        f,
        *args,
        cache_read=True,
        cache_write=True,
        aiocache_wait_for_write=True,
        **kwargs,
    ):
        key = self.get_cache_key(f, args, kwargs)

        if cache_read:
            value = await self.get_from_cache(key)
            if value is not None:
                return value

        if is_coroutine_callable(f):
            result = await f(*args, **kwargs)
        else:
            result = await run_in_threadpool(f, *args, **kwargs)

        if cache_write:
            if aiocache_wait_for_write:
                await self.set_in_cache(key, result)
            else:
                asyncio.ensure_future(self.set_in_cache(key, result))

        return result


def setup_cache():
    """Setup aiocache."""
    config: Dict[str, Any] = {
        "cache": "aiocache.RedisCache",
        "serializer": {"class": "aiocache.serializers.PickleSerializer"},
    }

    config["ttl"] = appconfig.TILE_CACHE_TTL

    url = urllib.parse.urlparse(
        f"redis://{appconfig.TILE_CACHE_URL}:{appconfig.TILE_CACHE_PORT}/0"
    )
    ulr_config = dict(urllib.parse.parse_qsl(url.query))
    config.update(ulr_config)

    cache_class = aiocache.Cache.get_scheme_class(url.scheme)
    config.update(cache_class.parse_uri_path(url.path))
    config["endpoint"] = url.hostname
    config["port"] = str(url.port)

    if url.password:
        config["password"] = url.password

    if cache_class == aiocache.Cache.REDIS:
        config["cache"] = "aiocache.RedisCache"
    elif cache_class == aiocache.Cache.MEMCACHED:
        config["cache"] = "aiocache.MemcachedCache"

    aiocache.caches.set_config({"default": config})

    # Test Redis connection
    try:
        client = redis.StrictRedis(
            host=config["endpoint"],
            port=config["port"],
            password=config.get("password", None),
        )
        client.ping()
        print("Redis connection successful")
    except redis.ConnectionError as e:
        print(f"Redis connection failed: {e}")

    print("Cache setup complete.")
