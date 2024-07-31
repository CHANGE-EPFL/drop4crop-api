from typing import Dict
from rio_tiler.models import ImageData
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    HTTPException,
    APIRouter,
    Query,
    BackgroundTasks,
    Request,
)
from app.config import config
from app.auth import require_admin, User
from botocore.exceptions import NoCredentialsError, PartialCredentialsError
from fastapi.responses import JSONResponse
from app.s3.services import get_s3
from titiler.core.factory import TilerFactory
from titiler.core.errors import DEFAULT_STATUS_CODES, add_exception_handlers
from rio_tiler.io import BaseReader
from rasterio.io import MemoryFile
import boto3
from rio_tiler.models import ImageData, Info
from morecantile import TileMatrixSet


class S3COGReader(BaseReader):
    def __init__(
        self,
        key: str,
        bucket: str = config.S3_BUCKET_ID,
        aws_access_key_id: str = config.S3_ACCESS_KEY,
        aws_secret_access_key: str = config.S3_SECRET_KEY,
        endpoint_url: str = f"https://{config.S3_URL}",
    ):
        self.bucket = bucket
        self.key = key
        self.aws_access_key_id = aws_access_key_id
        self.aws_secret_access_key = aws_secret_access_key
        self.endpoint_url = endpoint_url

    def fetch_cog(self):
        s3_client = boto3.client(
            "s3",
            aws_access_key_id=self.aws_access_key_id,
            aws_secret_access_key=self.aws_secret_access_key,
            endpoint_url=self.endpoint_url,
        )
        # s3_client = s3_target.client("s3")
        response = s3_client.get_object(Bucket=self.bucket, Key=self.key)
        # print(response)
        presigned_url = s3_client.generate_presigned_url(
            "get_object",
            Params={"Bucket": self.bucket, "Key": self.key},
            ExpiresIn=3600,
        )
        print(presigned_url)
        return
        cog_data = response["Body"].read()
        return cog_data

    def info(self):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                info = {
                    "bounds": dataset.bounds,
                    "minzoom": 0,
                    "maxzoom": 22,
                    "band_metadata": [],
                    "band_descriptions": [],
                    "nodata_type": (
                        "Nodata" if dataset.nodata is not None else "None"
                    ),
                    "dtype": str(dataset.dtypes[0]),
                    "colorinterp": [
                        str(interp) for interp in dataset.colorinterp
                    ],
                    "nodata_value": dataset.nodata,
                    "driver": dataset.driver,
                    "width": dataset.width,
                    "height": dataset.height,
                    "crs": dataset.crs.to_string(),
                }
                return Info(**info)

    def tile(self, x, y, z, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.tile(x, y, z, **kwargs)

    def part(self, bbox, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.part(bbox, **kwargs)

    def feature(self, shape: Dict, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.feature(shape, **kwargs)

    def preview(self, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.preview(**kwargs)

    def point(self, lon, lat, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.point(lon, lat, **kwargs)

    def statistics(self, **kwargs):
        with MemoryFile(self.fetch_cog()) as memfile:
            with memfile.open() as dataset:
                return dataset.statistics(**kwargs)


# Custom path dependency to use S3COGReader
def s3_cog_reader(
    key: str,
    tms: TileMatrixSet | None = None,
) -> S3COGReader:
    return S3COGReader(key=key)


router = APIRouter()

tiler = TilerFactory(reader=s3_cog_reader)

# cog = TilerFactory()
router.include_router(tiler.router)
