import aioboto3
import attr
import boto3
import warnings
import rasterio
from app.config import config
from app.db import get_session, AsyncSession
from app.layers.models import Layer
from app.s3.services import get_s3
from fastapi import HTTPException, Query, Depends
from rio_tiler.io import Reader
from rasterio.io import MemoryFile
from rio_tiler.errors import NoOverviewWarning
from sqlmodel import select
from titiler.core.factory import TilerFactory
from typing import Dict, Optional


@attr.s
class S3Reader(Reader):
    """Override the Reader class to call S3 directly."""

    def __attrs_post_init__(self):
        """Define _kwargs, open dataset and get info."""
        self.client = boto3.client(
            "s3",
            aws_access_key_id=config.S3_ACCESS_KEY,
            aws_secret_access_key=config.S3_SECRET_KEY,
            endpoint_url=f"https://{config.S3_URL}",
        )
        if not self.dataset:
            with MemoryFile(self._read(self.input)) as memfile:
                self.dataset = rasterio.open(memfile)

        self.bounds = tuple(self.dataset.bounds)
        self.crs = self.dataset.crs

        if self.colormap is None:
            self._get_colormap()

        if min(
            self.dataset.width, self.dataset.height
        ) > 512 and not self.dataset.overviews(1):
            warnings.warn(
                "The dataset has no Overviews.",
                NoOverviewWarning,
            )

    def _read(self, url: str):
        response = self.client.get_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{url}.tif",
        )
        return response["Body"].read()


async def ColorMapParams(
    s3: aioboto3.Session = Depends(get_s3),
    session: AsyncSession = Depends(get_session),
    url: str = Query(),
) -> Optional[Dict]:
    """Colormap Dependency."""

    query = select(Layer).where(Layer.layer_name == url)
    layer = await session.exec(query)
    layer = layer.one_or_none()

    if not layer:
        raise HTTPException(status_code=404, detail="Layer not found")

    if layer.style and layer.style.style:
        style = layer.style.style

        # Sort the style array by the 'value' field
        style_sorted = sorted(style, key=lambda x: x["value"])

        # Create the colormap with end values being the start of the next
        colormap = []
        for i in range(len(style_sorted)):
            start = style_sorted[i]["value"]
            end = (
                style_sorted[i + 1]["value"]
                if i + 1 < len(style_sorted)
                else start + 1.0
            )  # or some logical end value
            color = [
                style_sorted[i]["red"],
                style_sorted[i]["green"],
                style_sorted[i]["blue"],
                style_sorted[i]["opacity"],
            ]
            colormap.append([[start, end], color])

        return colormap

    # If no style is provided, use shades of grey
    min_value = layer.min_value
    max_value = layer.max_value

    num_segments = 10
    step = (max_value - min_value) / num_segments

    colormap = []
    for i in range(num_segments):
        start = min_value + i * step
        end = min_value + (i + 1) * step
        grey_value = int(255 * (i / (num_segments - 1)))
        color = [grey_value, grey_value, grey_value, 255]
        colormap.append([[start, end], color])

    return colormap


cog = TilerFactory(
    reader=S3Reader,
    colormap_dependency=ColorMapParams,
)
