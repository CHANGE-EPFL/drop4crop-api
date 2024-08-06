import aioboto3
import attr
import boto3
import warnings
import rasterio
from app.config import config
from app.db import get_session, AsyncSession
from app.layers.models import Layer
from app.s3.services import get_s3
from functools import lru_cache
from typing import Annotated, Optional, Dict, Literal
from pydantic import conint
from fastapi import HTTPException, Depends, Path, Query
from rio_tiler.io import Reader
from rasterio.io import MemoryFile
from rio_tiler.errors import NoOverviewWarning
from sqlmodel import select
from titiler.core.factory import img_endpoint_params
from dataclasses import dataclass
from starlette.responses import Response
from titiler.core.dependencies import ColorMapParams
from titiler.core.resources.enums import ImageType
from titiler.core.utils import render_image
from titiler.core.factory import TilerFactory as ParentTilerFactory
from cashews import cache

# Cache setup for the tile cache
cache.setup(
    f"redis://{config.TILE_CACHE_URL}:{config.TILE_CACHE_PORT}/",
    db=1,
    wait_for_connection_timeout=2.5,
    suppress=False,
    enable=True,
    timeout=10,
    client_side=True,
)


@dataclass
class TilerFactory(ParentTilerFactory):

    def tile(self):  # noqa: C901
        """Register /tiles endpoint."""

        @self.router.get(
            r"/tiles/{z}/{x}/{y}", **img_endpoint_params, deprecated=True
        )
        @self.router.get(
            r"/tiles/{z}/{x}/{y}.{format}",
            **img_endpoint_params,
            deprecated=True,
        )
        @self.router.get(
            r"/tiles/{z}/{x}/{y}@{scale}x",
            **img_endpoint_params,
            deprecated=True,
        )
        @self.router.get(
            r"/tiles/{z}/{x}/{y}@{scale}x.{format}",
            **img_endpoint_params,
            deprecated=True,
        )
        @self.router.get(
            r"/tiles/{tileMatrixSetId}/{z}/{x}/{y}", **img_endpoint_params
        )
        @self.router.get(
            r"/tiles/{tileMatrixSetId}/{z}/{x}/{y}.{format}",
            **img_endpoint_params,
        )
        @self.router.get(
            r"/tiles/{tileMatrixSetId}/{z}/{x}/{y}@{scale}x",
            **img_endpoint_params,
        )
        @self.router.get(
            r"/tiles/{tileMatrixSetId}/{z}/{x}/{y}@{scale}x.{format}",
            **img_endpoint_params,
        )
        @cache(ttl="3h", key="tile:{src_path}:{z}:{x}:{y}:{scale}:{format}")
        async def tile(
            z: Annotated[
                int,
                Path(
                    description="Identifier (Z) selecting one of the scales defined in the TileMatrixSet and representing the scaleDenominator the tile.",
                ),
            ],
            x: Annotated[
                int,
                Path(
                    description="Column (X) index of the tile on the selected TileMatrix. It cannot exceed the MatrixHeight-1 for the selected TileMatrix.",
                ),
            ],
            y: Annotated[
                int,
                Path(
                    description="Row (Y) index of the tile on the selected TileMatrix. It cannot exceed the MatrixWidth-1 for the selected TileMatrix.",
                ),
            ],
            tileMatrixSetId: Annotated[
                Literal[tuple(self.supported_tms.list())],
                f"Identifier selecting one of the TileMatrixSetId supported (default: '{self.default_tms}')",
            ] = self.default_tms,
            scale: Annotated[
                conint(gt=0, le=4), "Tile size scale. 1=256x256, 2=512x512..."
            ] = 1,
            format: Annotated[
                ImageType,
                "Default will be automatically defined if the output image needs a mask (png) or not (jpeg).",
            ] = None,
            src_path=Depends(self.path_dependency),
            layer_params=Depends(self.layer_dependency),
            dataset_params=Depends(self.dataset_dependency),
            tile_params=Depends(self.tile_dependency),
            post_process=Depends(self.process_dependency),
            rescale=Depends(self.rescale_dependency),
            color_formula=Depends(self.color_formula_dependency),
            colormap=Depends(self.colormap_dependency),
            render_params=Depends(self.render_dependency),
            reader_params=Depends(self.reader_dependency),
            env=Depends(self.environment_dependency),
        ):
            """Create map tile from a dataset."""
            tms = self.supported_tms.get(tileMatrixSetId)
            with rasterio.Env(**env):
                with self.reader(
                    src_path, tms=tms, **reader_params
                ) as src_dst:
                    image = src_dst.tile(
                        x,
                        y,
                        z,
                        tilesize=scale * 256,
                        **tile_params,
                        **layer_params,
                        **dataset_params,
                    )
                    dst_colormap = getattr(src_dst, "colormap", None)

            if post_process:
                image = post_process(image)

            if rescale:
                image.rescale(rescale)

            if color_formula:
                image.apply_color_formula(color_formula)

            content, media_type = render_image(
                image,
                output_format=format,
                colormap=colormap or dst_colormap,
                **render_params,
            )

            return Response(content, media_type=media_type)


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


@lru_cache(maxsize=128)
async def CachedColorMapParams(
    url: str,
    s3_session: aioboto3.Session,
    session: AsyncSession,
) -> Optional[Dict]:
    """Cached Colormap Dependency."""

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


async def ColorMapParams(
    s3: aioboto3.Session = Depends(get_s3),
    session: AsyncSession = Depends(get_session),
    url: str = Query(),
) -> Optional[Dict]:
    """Colormap Dependency with caching."""
    return await CachedColorMapParams(url, s3, session)


cog = TilerFactory(
    reader=S3Reader,
    colormap_dependency=ColorMapParams,
)
